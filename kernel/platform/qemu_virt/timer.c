// ARM Generic Timer backend for QEMU virt — ARM ARM D11.2
//
// Uses the EL1 Physical Timer (CNTP_*) system registers.
// The timer frequency is read from CNTFRQ_EL0 (set by firmware).

#include "../../../include/bazzulto/hal/hal_timer.h"
#include "../../../include/bazzulto/hal/hal_irq.h"
#include "../../../include/bazzulto/console.h"

// ARM Generic Timer system register accessors — ARM ARM D11.2

static inline uint64_t read_cntfrq(void)
{
    uint64_t val;
    __asm__ volatile("mrs %0, cntfrq_el0" : "=r"(val));
    return val;
}

static inline uint64_t read_cntpct(void)
{
    uint64_t val;
    __asm__ volatile("mrs %0, cntpct_el0" : "=r"(val));
    return val;
}

static inline void write_cntp_cval(uint64_t val)
{
    __asm__ volatile("msr cntp_cval_el0, %0" :: "r"(val));
}

static inline void write_cntp_ctl(uint64_t val)
{
    __asm__ volatile("msr cntp_ctl_el0, %0" :: "r"(val));
}

static uint64_t ticks_per_ms;

void hal_timer_init(void)
{
    // The GIC distributor and CPU interface are already initialized by
    // hal_irq_init(). We only need to program the timer itself.

    ticks_per_ms = read_cntfrq() / 1000;
    write_cntp_cval(read_cntpct() + ticks_per_ms * HAL_TIMER_TICK_MS);
    write_cntp_ctl(1);  // ENABLE=1, IMASK=0

    console_println("Timer: ok");
}

void hal_timer_handle_irq(void)
{
    // Program the next tick.
    write_cntp_cval(read_cntpct() + ticks_per_ms * HAL_TIMER_TICK_MS);
}

void hal_timer_delay_ms(uint32_t ms)
{
    uint64_t target = read_cntpct() + ticks_per_ms * ms;
    while (read_cntpct() < target)
        ;
}

uint64_t hal_timer_read_counter(void)
{
    return read_cntpct();
}

uint64_t hal_timer_read_frequency(void)
{
    return read_cntfrq();
}
