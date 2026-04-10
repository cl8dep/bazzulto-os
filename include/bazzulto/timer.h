#pragma once

#include <stdint.h>

// Initialize the GIC (Generic Interrupt Controller) and the ARM generic timer.
// After this call, a timer IRQ fires every TIMER_TICK_MS milliseconds.
void timer_init(void);

// Program the next timer tick. Called by the IRQ dispatcher when
// INTID 30 fires. Does NOT touch GICC_IAR/EOIR — that is the
// dispatcher's responsibility.
void timer_handle_irq(void);

// Milliseconds per scheduler tick.
#define TIMER_TICK_MS 10

// Busy-wait for the specified number of milliseconds.
// Uses the ARM generic timer counter (CNTPCT_EL0) for accurate timing.
void timer_delay_ms(uint32_t ms);
