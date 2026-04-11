#include "../../include/bazzulto/scheduler.h"
#include <string.h>
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/console.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/virtual_memory.h"
#include "../../include/bazzulto/kernel.h"
#include "../../include/bazzulto/virtual_file_system.h"
#include "../../include/bazzulto/pid.h"

// Defined in context_switch.S
extern void context_switch(cpu_context_t *from, cpu_context_t *to);
extern void process_entry_trampoline(void);
extern void process_entry_trampoline_user(void);
extern void fork_child_resume(void);

// User-space address layout
#define USER_TEXT_BASE   0x00400000ULL
#define USER_STACK_TOP   0x7FFFFFF000ULL

#define KERNEL_STACK_SIZE 24576  // 24KB — handles process + IRQ exception frame + full call chain with margin

static process_t *current      = NULL;  // process currently running
static process_t *run_queue    = NULL;  // head of the circular ready list

// A dummy context used as the "from" target when starting the first process —
// we need somewhere to save the bootstrap context even though we never return to it.
static cpu_context_t bootstrap_context;

void scheduler_init(void) {
    current   = NULL;
    run_queue = NULL;
    pid_init(physical_memory_total_bytes());
    console_println("Scheduler: initialized");
}

process_t *scheduler_create_process(void (*entry_point)(void)) {
    process_t *process = (process_t *)kmalloc(sizeof(process_t));
    if (!process) return NULL;

    uint8_t *stack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!stack) { kfree(process); return NULL; }

    process->pid              = pid_alloc();
    if (process->pid.index == 0) { kfree(stack); kfree(process); return NULL; }
    process->state            = PROCESS_STATE_READY;
    process->page_table       = NULL;  // shares kernel page table for now
    process->kernel_stack     = stack;
    process->next             = NULL;
    process->wait_next        = NULL;
    process->parent_pid       = 0;
    process->waiting_for_pid  = 0;
    process->exit_status      = 0;
    process->zombie_count     = 0;
    memset(process->mmap_regions, 0, sizeof(process->mmap_regions));
    process->mmap_next_vaddr  = MMAP_USER_BASE;
    virtual_file_system_init_fds(process->fds);

    // Set up the initial context so that when context_switch restores it,
    // execution begins at process_entry_trampoline (defined in context_switch.S).
    //
    // The trampoline enables IRQs then jumps to the real entry point (in x19).
    // This is necessary because context_switch itself does NOT unmask IRQs —
    // resumed processes rely on eret to restore SPSR (which has I=0). But a
    // fresh process has no exception frame, so it needs the trampoline.
    uint64_t stack_top = (uint64_t)(stack + KERNEL_STACK_SIZE);
    stack_top = stack_top & ~(uint64_t)15;  // AAPCS64 requires 16-byte SP alignment

    process->context.x19 = (uint64_t)entry_point;  // real entry, used by trampoline
    process->context.x20 = 0;
    process->context.x21 = 0;
    process->context.x22 = 0;
    process->context.x23 = 0;
    process->context.x24 = 0;
    process->context.x25 = 0;
    process->context.x26 = 0;
    process->context.x27 = 0;
    process->context.x28 = 0;
    process->context.x29 = 0;
    process->context.x30 = (uint64_t)process_entry_trampoline;
    process->context.sp  = stack_top;

    // Append to circular run queue
    if (!run_queue) {
        run_queue    = process;
        process->next = process;  // points to itself
    } else {
        // Find the last node and insert after it
        process_t *last = run_queue;
        while (last->next != run_queue) last = last->next;
        last->next    = process;
        process->next = run_queue;
    }

    return process;
}

// ---------------------------------------------------------------------------
// scheduler_free_user_process — release all memory held by a dead user process
//
// Walks the process's TTBR0 page table (L0→L1→L2→L3) and frees every mapped
// physical page, then frees every intermediate table page, then frees the
// kernel stack and the process struct itself.
//
// ARM ARM D5.2: a 4-level (L0-L3) table walk. Intermediate entries that are
// valid table descriptors have bits [1:0] = 0b11. Leaf (L3) entries have the
// same encoding but are reached at depth 3 — we free the physical page they
// point to. Intermediate table entries point to the physical address of the
// next-level table, which we also free after walking it.
//
// Only called for user processes (page_table != NULL). Kernel threads share
// the kernel page table and must never be passed here.
// ---------------------------------------------------------------------------
#define PAGE_TABLE_ENTRY_COUNT 512
#define PAGE_DESCRIPTOR_VALID  (1ULL << 0)
#define PAGE_DESCRIPTOR_TABLE  (1ULL << 1)
#define ENTRY_PHYSICAL_ADDRESS(e) ((e) & 0x0000FFFFFFFFF000ULL)

// Walk and free the user page table (L0-L3 + all mapped physical pages).
// Does NOT free the kernel stack or the process struct.
// Called by scheduler_free_user_address_space and scheduler_free_user_process.
static void free_user_page_table(uint64_t *l0_table)
{
    for (int l0 = 0; l0 < PAGE_TABLE_ENTRY_COUNT; l0++) {
        uint64_t l0_entry = l0_table[l0];
        if (!(l0_entry & PAGE_DESCRIPTOR_VALID))
            continue;

        uint64_t *l1_table = (uint64_t *)PHYSICAL_TO_VIRTUAL(
                                 ENTRY_PHYSICAL_ADDRESS(l0_entry));

        for (int l1 = 0; l1 < PAGE_TABLE_ENTRY_COUNT; l1++) {
            uint64_t l1_entry = l1_table[l1];
            if (!(l1_entry & PAGE_DESCRIPTOR_VALID))
                continue;

            uint64_t *l2_table = (uint64_t *)PHYSICAL_TO_VIRTUAL(
                                     ENTRY_PHYSICAL_ADDRESS(l1_entry));

            for (int l2 = 0; l2 < PAGE_TABLE_ENTRY_COUNT; l2++) {
                uint64_t l2_entry = l2_table[l2];
                if (!(l2_entry & PAGE_DESCRIPTOR_VALID))
                    continue;

                uint64_t *l3_table = (uint64_t *)PHYSICAL_TO_VIRTUAL(
                                         ENTRY_PHYSICAL_ADDRESS(l2_entry));

                for (int l3 = 0; l3 < PAGE_TABLE_ENTRY_COUNT; l3++) {
                    uint64_t l3_entry = l3_table[l3];
                    if (!(l3_entry & PAGE_DESCRIPTOR_VALID))
                        continue;
                    physical_memory_free((void *)ENTRY_PHYSICAL_ADDRESS(l3_entry));
                }
                physical_memory_free((void *)ENTRY_PHYSICAL_ADDRESS(l2_entry));
            }
            physical_memory_free((void *)ENTRY_PHYSICAL_ADDRESS(l1_entry));
        }
        physical_memory_free((void *)ENTRY_PHYSICAL_ADDRESS(l0_entry));
    }
    physical_memory_free((void *)VIRTUAL_TO_PHYSICAL(l0_table));
}

void scheduler_free_user_address_space(process_t *process)
{
    if (!process || !process->page_table) return;
    free_user_page_table(process->page_table);
    process->page_table = NULL;
}

void scheduler_free_user_process(process_t *process) {
    if (!process) return;

    if (!process->page_table) {
        // Kernel thread — no user page table to walk, just free stack + struct.
        kfree(process->kernel_stack);
        kfree(process);
        return;
    }

    free_user_page_table(process->page_table);
    process->page_table = NULL;

    kfree(process->kernel_stack);
    kfree(process);
}

process_t *scheduler_create_user_process(const void *code, size_t code_size) {
    process_t *process = (process_t *)kmalloc(sizeof(process_t));
    if (!process) return NULL;

    uint8_t *kstack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!kstack) { kfree(process); return NULL; }

    process->pid             = pid_alloc();
    if (process->pid.index == 0) { kfree(kstack); kfree(process); return NULL; }
    process->state           = PROCESS_STATE_READY;
    process->kernel_stack    = kstack;
    process->next            = NULL;
    process->wait_next       = NULL;
    process->parent_pid      = 0;
    process->waiting_for_pid = 0;
    process->exit_status     = 0;
    process->zombie_count    = 0;
    memset(process->mmap_regions, 0, sizeof(process->mmap_regions));
    process->mmap_next_vaddr = MMAP_USER_BASE;
    virtual_file_system_init_fds(process->fds);

    // --- Create per-process page table (TTBR0) ---
    uint64_t *user_table = virtual_memory_create_table();
    if (!user_table) { kfree(kstack); kfree(process); return NULL; }
    process->page_table = user_table;

    // --- Map user code at USER_TEXT_BASE ---
    size_t pages_needed = (code_size + PAGE_SIZE - 1) / PAGE_SIZE;
    for (size_t i = 0; i < pages_needed; i++) {
        void *phys = physical_memory_alloc();
        if (!phys) { kfree(kstack); kfree(process); return NULL; }
        uint8_t *virt = PHYSICAL_TO_VIRTUAL(phys);
        size_t offset = i * PAGE_SIZE;
        size_t chunk = code_size - offset;
        if (chunk > PAGE_SIZE) chunk = PAGE_SIZE;
        memcpy(virt, (const uint8_t *)code + offset, chunk);
        if (chunk < PAGE_SIZE) memset(virt + chunk, 0, PAGE_SIZE - chunk);
        virtual_memory_map(user_table, USER_TEXT_BASE + offset,
                           (uint64_t)phys, PAGE_FLAGS_USER_CODE);
    }

    // --- Map user stack (one page below USER_STACK_TOP) ---
    void *stack_phys = physical_memory_alloc();
    if (!stack_phys) { kfree(kstack); kfree(process); return NULL; }
    memset(PHYSICAL_TO_VIRTUAL(stack_phys), 0, PAGE_SIZE);
    virtual_memory_map(user_table, USER_STACK_TOP - PAGE_SIZE,
                       (uint64_t)stack_phys, PAGE_FLAGS_USER_DATA);

    // --- Set initial context for EL0 entry ---
    uint64_t kstack_top = (uint64_t)(kstack + KERNEL_STACK_SIZE) & ~(uint64_t)15;
    process->context.x19 = USER_TEXT_BASE;       // user entry point
    process->context.x20 = USER_STACK_TOP;       // user stack top
    process->context.x21 = 0;
    process->context.x22 = 0;
    process->context.x23 = 0;
    process->context.x24 = 0;
    process->context.x25 = 0;
    process->context.x26 = 0;
    process->context.x27 = 0;
    process->context.x28 = 0;
    process->context.x29 = 0;
    process->context.x30 = (uint64_t)process_entry_trampoline_user;
    process->context.sp  = kstack_top;           // kernel stack for exceptions

    // Append to circular run queue
    if (!run_queue) {
        run_queue = process;
        process->next = process;
    } else {
        process_t *last = run_queue;
        while (last->next != run_queue) last = last->next;
        last->next    = process;
        process->next = run_queue;
    }

    return process;
}

process_t *scheduler_create_user_process_from_image(uint64_t *page_table,
                                                     uint64_t entry_point,
                                                     uint64_t user_stack_top) {
    process_t *process = (process_t *)kmalloc(sizeof(process_t));
    if (!process) return NULL;

    uint8_t *kstack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!kstack) { kfree(process); return NULL; }

    process->pid             = pid_alloc();
    if (process->pid.index == 0) { kfree(kstack); kfree(process); return NULL; }
    process->state           = PROCESS_STATE_READY;
    process->page_table      = page_table;
    process->kernel_stack    = kstack;
    process->next            = NULL;
    process->wait_next       = NULL;
    process->parent_pid      = 0;
    process->waiting_for_pid = 0;
    process->exit_status     = 0;
    process->zombie_count    = 0;
    memset(process->mmap_regions, 0, sizeof(process->mmap_regions));
    process->mmap_next_vaddr = MMAP_USER_BASE;
    virtual_file_system_init_fds(process->fds);

    // Set initial context for EL0 entry via process_entry_trampoline_user.
    // The trampoline writes ELR_EL1 = x19, SPSR_EL1 = 0 (EL0t), SP_EL0 = x20,
    // zeroes all GPRs, then performs eret.
    uint64_t kstack_top = (uint64_t)(kstack + KERNEL_STACK_SIZE) & ~(uint64_t)15;
    process->context.x19 = entry_point;
    process->context.x20 = user_stack_top;
    process->context.x21 = 0;
    process->context.x22 = 0;
    process->context.x23 = 0;
    process->context.x24 = 0;
    process->context.x25 = 0;
    process->context.x26 = 0;
    process->context.x27 = 0;
    process->context.x28 = 0;
    process->context.x29 = 0;
    process->context.x30 = (uint64_t)process_entry_trampoline_user;
    process->context.sp  = kstack_top;

    // Append to circular run queue
    if (!run_queue) {
        run_queue = process;
        process->next = process;
    } else {
        process_t *last = run_queue;
        while (last->next != run_queue) last = last->next;
        last->next    = process;
        process->next = run_queue;
    }

    return process;
}

// Switch TTBR0 if the next process has a different page table.
static void switch_address_space(process_t *prev, process_t *next) {
    if (next->page_table && next->page_table != prev->page_table)
        virtual_memory_switch_ttbr0(next->page_table);
}

void scheduler_tick(void) {
    if (!current || !current->next) return;

    process_t *previous = current;
    current = current->next;

    // Skip processes that are not schedulable (blocked, waiting, zombie, dead).
    while (current->state != PROCESS_STATE_READY &&
           current->state != PROCESS_STATE_RUNNING) {
        current = current->next;
        if (current == previous) return;  // no runnable process found
    }

    if (current == previous) return;  // only one process, nothing to switch

    previous->state = PROCESS_STATE_READY;
    current->state  = PROCESS_STATE_RUNNING;

    switch_address_space(previous, current);
    context_switch(&previous->context, &current->context);
}

process_t *scheduler_get_current(void) {
    return current;
}

void scheduler_yield(void) {
    if (!current) return;

    process_t *prev = current;
    process_t *next = current->next;

    // Find the next READY process in the circular list.
    while (next->state != PROCESS_STATE_READY) {
        next = next->next;
        if (next == prev) {
            // No runnable process. Spin with IRQs enabled until an IRQ
            // handler wakes someone up (e.g. UART RX wakes a reader).
            __asm__ volatile("msr daifclr, #2");
            while (1) {
                __asm__ volatile("wfi");
                // Re-scan after IRQ returns
                process_t *scan = prev->next;
                do {
                    if (scan->state == PROCESS_STATE_READY) {
                        next = scan;
                        goto found;
                    }
                    scan = scan->next;
                } while (scan != prev->next);
            }
        found:
            __asm__ volatile("msr daifset, #2");
            break;
        }
    }

    current = next;
    current->state = PROCESS_STATE_RUNNING;
    switch_address_space(prev, current);
    context_switch(&prev->context, &current->context);
    // Returns here when `prev` is eventually switched back to.
}

process_t *scheduler_find_process(uint16_t pid_index)
{
    if (!run_queue) return NULL;

    process_t *p = run_queue;
    do {
        if (p->pid.index == pid_index) return p;
        p = p->next;
    } while (p != run_queue);

    return NULL;
}

void scheduler_wake_waiters(uint16_t pid_index)
{
    if (!run_queue) return;

    process_t *p = run_queue;
    do {
        if (p->state == PROCESS_STATE_WAITING) {
            // Wake a process waiting for this specific PID.
            if (p->waiting_for_pid == pid_index) {
                p->state = PROCESS_STATE_READY;
            }
            // Wake a process waiting for ANY child (wait(-1) sentinel 0xFFFF)
            // if the dying process is actually a child of the waiter.
            else if (p->waiting_for_pid == 0xFFFF) {
                process_t *dying = scheduler_find_process(pid_index);
                if (dying && dying->parent_pid == p->pid.index)
                    p->state = PROCESS_STATE_READY;
            }
        }
        p = p->next;
    } while (p != run_queue);
}

process_t *scheduler_find_zombie_child(uint16_t parent_pid)
{
    if (!run_queue) return NULL;

    process_t *p = run_queue;
    do {
        if (p->parent_pid == parent_pid && p->state == PROCESS_STATE_ZOMBIE)
            return p;
        p = p->next;
    } while (p != run_queue);

    return NULL;
}

int scheduler_has_child(uint16_t parent_pid)
{
    if (!run_queue) return 0;

    process_t *p = run_queue;
    do {
        if (p->parent_pid == parent_pid)
            return 1;
        p = p->next;
    } while (p != run_queue);

    return 0;
}

void scheduler_reparent_children(uint16_t dying_pid, uint16_t init_pid)
{
    if (!run_queue) return;

    // Reassign every process whose parent is the dying process to init (PID 1).
    // This prevents their zombie entries from leaking when the dying process
    // is reaped and no one else can call wait() for them.
    process_t *p = run_queue;
    do {
        if (p->parent_pid == dying_pid)
            p->parent_pid = init_pid;
        p = p->next;
    } while (p != run_queue);
}

void scheduler_reap_process(process_t *process)
{
    if (!process || !run_queue) return;

    // Reaping the currently running process would corrupt the scheduler state —
    // the caller must yield first (e.g. sys_exit sets ZOMBIE then calls
    // scheduler_yield before any reaper can call scheduler_reap_process).
    if (process == current) return;

    // Remove process from the circular run queue.
    if (process->next == process) {
        // Only node — queue becomes empty.
        run_queue = NULL;
    } else {
        // Find the predecessor and relink around process.
        process_t *predecessor = process;
        while (predecessor->next != process)
            predecessor = predecessor->next;
        predecessor->next = process->next;
        if (run_queue == process)
            run_queue = process->next;
    }

    // Release the PID index so it can be reused.
    pid_free(process->pid);

    // Free all memory (page tables, kernel stack, struct).
    scheduler_free_user_process(process);
}

// ---------------------------------------------------------------------------
// scheduler_fork_process — create a copy of the current process
// ---------------------------------------------------------------------------

uint16_t scheduler_fork_process(struct exception_frame *parent_frame)
{
    process_t *parent = current;

    // Allocate child struct + kernel stack.
    process_t *child = (process_t *)kmalloc(sizeof(process_t));
    if (!child) return 0;

    uint8_t *kstack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!kstack) { kfree(child); return 0; }

    // Assign a new PID.
    child->pid = pid_alloc();
    if (child->pid.index == 0) { kfree(kstack); kfree(child); return 0; }

    // Deep-copy the parent's user address space.
    // Every L3 leaf page gets its own fresh physical page.
    uint64_t *child_page_table = virtual_memory_deep_copy_table(parent->page_table);
    if (!child_page_table) { pid_free(child->pid); kfree(kstack); kfree(child); return 0; }

    // Copy parent's process fields.
    child->state           = PROCESS_STATE_READY;
    child->page_table      = child_page_table;
    child->kernel_stack    = kstack;
    child->parent_pid      = parent->pid.index;
    child->waiting_for_pid = 0;
    child->exit_status     = 0;
    child->zombie_count    = 0;
    child->mmap_next_vaddr = parent->mmap_next_vaddr;
    child->next            = NULL;
    child->wait_next       = NULL;

    // Copy the mmap region table so the child tracks its own mappings.
    memcpy(child->mmap_regions, parent->mmap_regions, sizeof(parent->mmap_regions));

    // Copy file descriptor table (pipes get their ref_count incremented).
    for (int i = 0; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
        child->fds[i] = parent->fds[i];
        if (child->fds[i].type == FD_TYPE_PIPE_READ ||
            child->fds[i].type == FD_TYPE_PIPE_WRITE) {
            if (child->fds[i].pipe)
                child->fds[i].pipe->ref_count++;
        }
    }

    // Copy the parent's exception frame onto the TOP of the child's kernel stack.
    // The frame lives at kstack + KERNEL_STACK_SIZE - 288.
    uint8_t *kstack_top = kstack + KERNEL_STACK_SIZE;
    struct exception_frame *child_frame =
        (struct exception_frame *)(kstack_top - sizeof(struct exception_frame));
    memcpy(child_frame, parent_frame, sizeof(struct exception_frame));

    // The child returns 0 from fork().
    child_frame->x0 = 0;

    // Set up the child's saved CPU context so that when context_switch
    // switches to it for the first time:
    //   - SP is restored to child_frame (bottom of the exception frame)
    //   - x30 (LR / ret address) is fork_child_resume
    //   - fork_child_resume expands restore_exception_frame_el0 → eret to EL0
    child->context.sp  = (uint64_t)child_frame;
    child->context.x30 = (uint64_t)fork_child_resume;

    // Zero out callee-saved registers so the child starts with a clean kernel state.
    child->context.x19 = 0;
    child->context.x20 = 0;
    child->context.x21 = 0;
    child->context.x22 = 0;
    child->context.x23 = 0;
    child->context.x24 = 0;
    child->context.x25 = 0;
    child->context.x26 = 0;
    child->context.x27 = 0;
    child->context.x28 = 0;
    child->context.x29 = 0;

    // Add child to the run queue.
    if (!run_queue) {
        child->next = child;
        run_queue = child;
    } else {
        process_t *tail = run_queue;
        while (tail->next != run_queue) tail = tail->next;
        tail->next  = child;
        child->next = run_queue;
    }

    return child->pid.index;
}

void scheduler_start(void) {
    if (!run_queue) {
        console_println("Scheduler: no processes to run");
        return;
    }

    current        = run_queue;
    current->state = PROCESS_STATE_RUNNING;

    // If the first process is a user process, load its TTBR0 page table now.
    // scheduler_tick/scheduler_yield call switch_address_space, but the very
    // first context_switch has no "previous" process — so we must switch here.
    if (current->page_table)
        virtual_memory_switch_ttbr0(current->page_table);

    // Switch from the bootstrap context into the first process.
    // context_switch will enable IRQs (msr daifclr, #2) before ret.
    // We pass bootstrap_context as `from` — it gets written but never read.
    context_switch(&bootstrap_context, &current->context);

    // Never reached
}
