#include "../../include/bazzulto/scheduler.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/console.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/virtual_memory.h"
#include "../../include/bazzulto/kernel.h"
#include "../../include/bazzulto/virtual_file_system.h"

// Defined in context_switch.S
extern void context_switch(cpu_context_t *from, cpu_context_t *to);
extern void process_entry_trampoline(void);
extern void process_entry_trampoline_user(void);

// User-space address layout
#define USER_TEXT_BASE   0x00400000ULL
#define USER_STACK_TOP   0x7FFFFFF000ULL

#define KERNEL_STACK_SIZE 16384  // 16KB — must handle process + IRQ exception frame + full call chain

static uint32_t   next_pid     = 1;
static process_t *current      = NULL;  // process currently running
static process_t *run_queue    = NULL;  // head of the circular ready list

// A dummy context used as the "from" target when starting the first process —
// we need somewhere to save the bootstrap context even though we never return to it.
static cpu_context_t bootstrap_context;

void scheduler_init(void) {
    current   = NULL;
    run_queue = NULL;
    console_println("Scheduler: initialized");
}

process_t *scheduler_create_process(void (*entry_point)(void)) {
    process_t *process = (process_t *)kmalloc(sizeof(process_t));
    if (!process) return NULL;

    uint8_t *stack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!stack) { kfree(process); return NULL; }

    process->pid              = next_pid++;
    process->state            = PROCESS_STATE_READY;
    process->page_table       = NULL;  // shares kernel page table for now
    process->kernel_stack     = stack;
    process->next             = NULL;
    process->wait_next        = NULL;
    process->waiting_for_pid  = 0;
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

static void memory_copy(void *dst, const void *src, size_t n) {
    uint8_t *d = dst;
    const uint8_t *s = src;
    for (size_t i = 0; i < n; i++) d[i] = s[i];
}

static void memory_zero(void *dst, size_t n) {
    uint8_t *d = dst;
    for (size_t i = 0; i < n; i++) d[i] = 0;
}

process_t *scheduler_create_user_process(const void *code, size_t code_size) {
    process_t *process = (process_t *)kmalloc(sizeof(process_t));
    if (!process) return NULL;

    uint8_t *kstack = (uint8_t *)kmalloc(KERNEL_STACK_SIZE);
    if (!kstack) { kfree(process); return NULL; }

    process->pid             = next_pid++;
    process->state           = PROCESS_STATE_READY;
    process->kernel_stack    = kstack;
    process->next            = NULL;
    process->wait_next       = NULL;
    process->waiting_for_pid = 0;
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
        memory_copy(virt, (const uint8_t *)code + offset, chunk);
        if (chunk < PAGE_SIZE) memory_zero(virt + chunk, PAGE_SIZE - chunk);
        virtual_memory_map(user_table, USER_TEXT_BASE + offset,
                           (uint64_t)phys, PAGE_FLAGS_USER_CODE);
    }

    // --- Map user stack (one page below USER_STACK_TOP) ---
    void *stack_phys = physical_memory_alloc();
    if (!stack_phys) { kfree(kstack); kfree(process); return NULL; }
    memory_zero(PHYSICAL_TO_VIRTUAL(stack_phys), PAGE_SIZE);
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

    process->pid             = next_pid++;
    process->state           = PROCESS_STATE_READY;
    process->page_table      = page_table;
    process->kernel_stack    = kstack;
    process->next            = NULL;
    process->wait_next       = NULL;
    process->waiting_for_pid = 0;
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

    // Skip processes that are not ready (blocked or dead)
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

process_t *scheduler_find_process(uint32_t pid)
{
    if (!run_queue) return NULL;

    process_t *p = run_queue;
    do {
        if (p->pid == pid) return p;
        p = p->next;
    } while (p != run_queue);

    return NULL;
}

void scheduler_wake_waiters(uint32_t pid)
{
    if (!run_queue) return;

    process_t *p = run_queue;
    do {
        if (p->state == PROCESS_STATE_WAITING && p->waiting_for_pid == pid) {
            p->state = PROCESS_STATE_READY;
        }
        p = p->next;
    } while (p != run_queue);
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
