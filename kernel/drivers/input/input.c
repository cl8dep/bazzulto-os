#include "../../../include/bazzulto/input.h"
#include "../../../include/bazzulto/waitqueue.h"
#include "../../../include/bazzulto/scheduler.h"
#include "../../../include/bazzulto/hal/hal_uart.h"

// ---------------------------------------------------------------------------
// Input ring buffer — single-producer (IRQ handlers) / single-consumer
// (input_getchar). All sources of stdin write here via input_emit_char().
// ---------------------------------------------------------------------------

#define INPUT_RING_SIZE 64

static char          input_ring[INPUT_RING_SIZE];
static volatile uint32_t input_ring_head;  // ISR writes here
static volatile uint32_t input_ring_tail;  // reader reads here
static wait_queue_t  input_wait_queue = WAIT_QUEUE_INIT;

void input_init(void)
{
    input_ring_head = 0;
    input_ring_tail = 0;
}

// Called from any IRQ handler that produces a character (keyboard, UART, etc.).
void input_emit_char(char character)
{
    // Ctrl+C (0x03): send SIGINT to the foreground process and echo "^C".
    // Do not place the character in the ring buffer; it is a control signal,
    // not data the reading process should see.
    if (character == 0x03) {
        hal_uart_putc('^');
        hal_uart_putc('C');
        hal_uart_putc('\r');
        hal_uart_putc('\n');
        scheduler_send_signal_to_foreground(2 /* SIGINT */);
        return;
    }

    uint32_t next_head = (input_ring_head + 1) % INPUT_RING_SIZE;
    if (next_head == input_ring_tail)
        return;  // ring full — drop character (preferred over blocking in IRQ context)

    input_ring[input_ring_head] = character;
    input_ring_head = next_head;

    process_wakeup(&input_wait_queue);
}

// Called by the VFS console read path. Blocks until a character is available.
// Returns the character (0–255) on success, or -1 if a signal is pending
// (so the caller can return from the syscall and deliver the signal).
int input_getchar(void)
{
    // Disable IRQs to prevent a race between checking the ring and going to
    // sleep — the same pattern used by uart_getc().
    __asm__ volatile("msr daifset, #2");

    while (input_ring_head == input_ring_tail) {
        // Check for pending signals before sleeping again.
        // Without this, a process blocked on console input would never see
        // a signal (e.g. SIGINT from Ctrl+C) because signals are only
        // delivered when the syscall handler returns to EL0.
        process_t *proc = scheduler_get_current();
        if (proc && proc->pending_signals) {
            __asm__ volatile("msr daifclr, #2");
            return -1;  // interrupted — let the syscall layer deliver the signal
        }
        process_sleep(&input_wait_queue);
        // process_sleep yields and returns when woken. Re-disable IRQs for
        // the next loop iteration.
        __asm__ volatile("msr daifset, #2");
    }

    char character = input_ring[input_ring_tail];
    input_ring_tail = (input_ring_tail + 1) % INPUT_RING_SIZE;

    __asm__ volatile("msr daifclr, #2");
    return (unsigned char)character;
}
