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

// Read the current physical counter value (CNTPCT_EL0).
// ARM ARM D11.2: accessible from EL0 and above.
// Frequency is given by timer_read_cntfrq().
uint64_t timer_read_cntpct(void);

// Read the counter frequency in Hz (CNTFRQ_EL0).
// ARM ARM D11.2: set by firmware at boot; fixed for the platform lifetime.
uint64_t timer_read_cntfrq(void);

// POSIX-compatible time specification.
// Used by SYSTEMCALL_CLOCK_GETTIME and SYSTEMCALL_NANOSLEEP.
struct timespec {
    int64_t tv_sec;   // seconds
    int64_t tv_nsec;  // nanoseconds [0, 999999999]
};
