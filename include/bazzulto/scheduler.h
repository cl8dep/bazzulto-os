#pragma once

#include <stdint.h>
#include <stddef.h>

// Process states
typedef enum {
    PROCESS_STATE_READY,    // waiting to be scheduled
    PROCESS_STATE_RUNNING,  // currently executing on the CPU
    PROCESS_STATE_BLOCKED,  // waiting for I/O or an event
    PROCESS_STATE_DEAD,     // finished, waiting to be cleaned up
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

typedef struct process process_t;

struct process {
    uint32_t        pid;
    process_state_t state;
    cpu_context_t   context;       // saved registers when not running
    uint64_t       *page_table;    // this process's virtual address space
    uint8_t        *kernel_stack;  // kernel stack base (allocated on creation)
    process_t      *next;          // next process in the circular run queue
    process_t      *wait_next;     // next process in a wait queue (NULL if not waiting)
};

// Initialize the scheduler. Must be called after heap and exceptions are ready.
void scheduler_init(void);

// Create a new process that will start executing at `entry_point`.
// Returns the new process, or NULL on allocation failure.
process_t *scheduler_create_process(void (*entry_point)(void));

// Called from the timer IRQ handler on every tick.
// Saves the current process state and switches to the next one.
void scheduler_tick(void);

// Start the scheduler — loads and runs the first process. Does not return.
void scheduler_start(void);

// Return a pointer to the currently running process.
process_t *scheduler_get_current(void);

// Voluntarily yield the CPU to the next ready process.
// Called when the current process blocks (e.g. waiting for I/O).
// The caller must have set current->state to BLOCKED before calling.
// IRQs must be disabled by the caller.
void scheduler_yield(void);
