#include "../../include/bazzulto/scheduler.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/console.h"

// Defined in context_switch.S
extern void context_switch(cpu_context_t *from, cpu_context_t *to);
extern void process_entry_trampoline(void);

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

    process->pid          = next_pid++;
    process->state        = PROCESS_STATE_READY;
    process->page_table   = NULL;  // shares kernel page table for now
    process->kernel_stack = stack;
    process->next         = NULL;

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
    context_switch(&prev->context, &current->context);
    // Returns here when `prev` is eventually switched back to.
}

void scheduler_start(void) {
    if (!run_queue) {
        console_println("Scheduler: no processes to run");
        return;
    }

    current        = run_queue;
    current->state = PROCESS_STATE_RUNNING;

    // Switch from the bootstrap context into the first process.
    // context_switch will enable IRQs (msr daifclr, #2) before ret.
    // We pass bootstrap_context as `from` — it gets written but never read.
    context_switch(&bootstrap_context, &current->context);

    // Never reached
}
