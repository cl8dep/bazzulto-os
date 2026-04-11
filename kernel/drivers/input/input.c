#include "../../../include/bazzulto/input.h"
#include "../../../include/bazzulto/waitqueue.h"

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
    uint32_t next_head = (input_ring_head + 1) % INPUT_RING_SIZE;
    if (next_head == input_ring_tail)
        return;  // ring full — drop character (preferred over blocking in IRQ context)

    input_ring[input_ring_head] = character;
    input_ring_head = next_head;

    process_wakeup(&input_wait_queue);
}

// Called by the VFS console read path. Blocks until a character is available.
char input_getchar(void)
{
    // Disable IRQs to prevent a race between checking the ring and going to
    // sleep — the same pattern used by uart_getc().
    __asm__ volatile("msr daifset, #2");

    while (input_ring_head == input_ring_tail) {
        process_sleep(&input_wait_queue);
        // process_sleep yields and returns when woken. Re-disable IRQs for
        // the next loop iteration.
        __asm__ volatile("msr daifset, #2");
    }

    char character = input_ring[input_ring_tail];
    input_ring_tail = (input_ring_tail + 1) % INPUT_RING_SIZE;

    __asm__ volatile("msr daifclr, #2");
    return character;
}
