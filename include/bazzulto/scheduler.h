#pragma once

#include <stdint.h>
#include <stddef.h>
#include "pid.h"
#include "virtual_file_system.h"
#include "exceptions.h"

// Process states
typedef enum {
    PROCESS_STATE_READY,    // waiting to be scheduled
    PROCESS_STATE_RUNNING,  // currently executing on the CPU
    PROCESS_STATE_BLOCKED,  // waiting for I/O or an event
    PROCESS_STATE_WAITING,  // blocked in wait() until a specific child exits
    PROCESS_STATE_ZOMBIE,   // exited, preserving exit_status until parent calls wait()
    PROCESS_STATE_DEAD,     // wait() collected exit status — safe to free
} process_state_t;

// Saved CPU state for a process that is not currently running.
// When a context switch occurs, registers are pushed onto the process stack
// and this struct records where that stack is.
typedef struct {
    uint64_t x19, x20, x21, x22, x23, x24, x25, x26, x27, x28;
    uint64_t x29;  // frame pointer
    uint64_t x30;  // link register (return address)
    uint64_t sp;   // stack pointer at time of switch
} cpu_context_t;

// Maximum anonymous mmap regions per process.
// Each sys_mmap call that succeeds consumes one slot; sys_munmap frees it.
#define PROCESS_MMAP_MAX_REGIONS 16

// One anonymous memory region tracked per process for munmap.
typedef struct {
    uint64_t vaddr;   // base virtual address (page-aligned)
    uint64_t n_pages; // number of pages in this region (0 = slot unused)
} mmap_region_t;

// Base virtual address for user anonymous mappings (above 8 GB).
// Chosen to be well above typical ELF load addresses (0x400000)
// and the user stack (0x7FFFF00000).
#define MMAP_USER_BASE 0x200000000ULL

typedef struct process process_t;

struct process {
    pid_t           pid;
    process_state_t state;
    cpu_context_t   context;            // saved registers when not running
    uint64_t       *page_table;         // this process's virtual address space
    uint8_t        *kernel_stack;       // kernel stack base (allocated on creation)
    file_descriptor_t fds[VIRTUAL_FILE_SYSTEM_MAX_FDS]; // per-process file descriptor table
    uint16_t        parent_pid;         // PID index of the parent (0 = no parent / init orphan)
    uint16_t        waiting_for_pid;    // PID index this process is blocked on (0 = not waiting)
                                        // 0xFFFF = waiting for ANY child (wait(-1))
    int             exit_status;        // exit code passed to sys_exit; valid when ZOMBIE
    uint16_t        zombie_count;       // number of un-reaped zombie children; capped at ZOMBIE_COUNT_MAX
    // Anonymous mmap regions. Bump-pointer within user VA space starting at MMAP_USER_BASE.
    mmap_region_t   mmap_regions[PROCESS_MMAP_MAX_REGIONS];
    uint64_t        mmap_next_vaddr;    // next free VA for anonymous mmap allocations
    // Signal state. pending_signals is a bitmask; bit N set = signal N is pending.
    // signal_handlers[N]: 0 = SIG_DFL (default), 1 = SIG_IGN, else = user handler VA.
    uint64_t        pending_signals;
    uint64_t        signal_handlers[32];
    process_t      *next;               // next process in the circular run queue
    process_t      *wait_next;          // next process in a wait queue (NULL if not waiting)
};

// Maximum un-reaped zombie children per process before sys_spawn starts refusing.
#define ZOMBIE_COUNT_MAX 64

// Initialize the scheduler. Must be called after heap and exceptions are ready.
void scheduler_init(void);

// Create a new kernel thread that will start executing at `entry_point`.
// Returns the new process, or NULL on allocation failure.
process_t *scheduler_create_process(void (*entry_point)(void));

// Create a new user-mode process. Copies `code_size` bytes from `code`
// into a new address space mapped at 0x00400000, allocates a user stack,
// and configures the process to eret to EL0 on first switch.
process_t *scheduler_create_user_process(const void *code, size_t code_size);

// Create a user-mode process from a pre-built page table.
// Used by the ELF loader, which maps segments and the user stack itself.
// The caller provides:
//   page_table       — fully populated TTBR0 table (code + stack already mapped)
//   entry_point      — virtual address where execution begins (ELF e_entry)
//   user_stack_top   — top of the user stack (mapped by the caller)
// Returns the new process, or NULL on allocation failure.
process_t *scheduler_create_user_process_from_image(uint64_t *page_table,
                                                     uint64_t entry_point,
                                                     uint64_t user_stack_top);

// Called from the timer IRQ handler on every tick.
// Saves the current process state and switches to the next one.
void scheduler_tick(void);

// Start the scheduler — loads and runs the first process. Does not return.
void scheduler_start(void);

// Return a pointer to the currently running process.
process_t *scheduler_get_current(void);

// Free all memory held by a dead user process: mapped physical pages,
// page table pages, kernel stack, and the process struct itself.
// Only valid for user processes (process->page_table != NULL).
// Must be called after the process has been removed from the run queue
// and will never be context-switched to again.
void scheduler_free_user_process(process_t *process);

// Voluntarily yield the CPU to the next ready process.
// Called when the current process blocks (e.g. waiting for I/O).
// The caller must have set current->state to BLOCKED before calling.
// IRQs must be disabled by the caller.
void scheduler_yield(void);

// Search the run queue for a process with the given PID index.
// Returns a pointer to the process, or NULL if not found.
process_t *scheduler_find_process(uint16_t pid_index);

// Wake all processes that are blocked in PROCESS_STATE_WAITING for pid_index.
// Called by sys_exit when a process dies so its waiters can be rescheduled.
void scheduler_wake_waiters(uint16_t pid_index);

// Reparent all processes whose parent_pid == dying_pid to init_pid (PID 1).
// Called by sys_exit so orphaned children get reaped by PID 1 instead of
// leaking as permanent zombies.
void scheduler_reparent_children(uint16_t dying_pid, uint16_t init_pid);

// Remove a ZOMBIE or DEAD process from the run queue and free its memory.
// Called by sys_wait after collecting the exit status.
void scheduler_reap_process(process_t *process);

// Return the first zombie child of parent_pid, or NULL if none exists.
// Used by sys_wait(-1) to find any exited child.
process_t *scheduler_find_zombie_child(uint16_t parent_pid);

// Return 1 if any process has parent_pid as its parent, 0 otherwise.
// Used by sys_wait(-1) to detect whether there are children to wait for.
int scheduler_has_child(uint16_t parent_pid);

// Free the user address space (TTBR0 page table and all mapped physical pages)
// of the given process without freeing the process struct itself or the kernel
// stack. Used by sys_exec to replace the image while keeping the PID, kernel
// stack, and file descriptors intact.
// Only valid for user processes (process->page_table != NULL).
void scheduler_free_user_address_space(process_t *process);

// Fork the current process.
// Allocates a new process_t, deep-copies the parent's user address space
// (all L3 leaf pages), copies the parent's exception frame onto the child's
// kernel stack with x0=0 (child returns 0 from fork), and adds the child to
// the run queue.
//
// parent_frame  — pointer to the parent's exception_frame on its kernel stack
//                 (saved by save_exception_frame_el0 before the SVC dispatch)
//
// Returns the child PID index (> 0) on success, 0 on failure.
// The parent's syscall return value (frame->x0) is set to the child PID
// by sys_fork before this function returns.
uint16_t scheduler_fork_process(struct exception_frame *parent_frame);
