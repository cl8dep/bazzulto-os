#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: System Timer
//
// Platform-independent interface for the system tick timer and monotonic clock.
// The platform backend (e.g. ARM generic timer for QEMU virt) implements these.
// ---------------------------------------------------------------------------

// Scheduler tick interval in milliseconds.
#define HAL_TIMER_TICK_MS 10

// Initialize the system timer. Programs the first tick and enables the
// timer interrupt via hal_irq_enable().
void hal_timer_init(void);

// Handle the timer interrupt. Reprograms the comparator for the next tick.
// Called from the IRQ dispatcher when HAL_IRQ_TIMER fires.
void hal_timer_handle_irq(void);

// Busy-wait for the specified number of milliseconds.
void hal_timer_delay_ms(uint32_t ms);

// Read the monotonic hardware counter (platform-specific units).
// Use hal_timer_read_frequency() to convert to seconds.
uint64_t hal_timer_read_counter(void);

// Read the counter frequency in Hz.
uint64_t hal_timer_read_frequency(void);

// POSIX-compatible time specification.
// Used by SYSTEMCALL_CLOCK_GETTIME and SYSTEMCALL_NANOSLEEP.
struct timespec {
    int64_t tv_sec;   // seconds
    int64_t tv_nsec;  // nanoseconds [0, 999999999]
};
