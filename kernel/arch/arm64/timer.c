#include "../../../include/bazzulto/timer.h"
#include "../../../include/bazzulto/console.h"
#include "../../../include/bazzulto/gic.h"

// ARM Generic Timer system register accessors — ARM ARM D11.2
static inline uint64_t read_cntfrq(void) {
	uint64_t val;
	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(val));
	return val;
}

static inline uint64_t read_cntpct(void) {
	uint64_t val;
	__asm__ volatile("mrs %0, cntpct_el0" : "=r"(val));
	return val;
}

static inline void write_cntp_cval(uint64_t val) {
	__asm__ volatile("msr cntp_cval_el0, %0" :: "r"(val));
}

static inline void write_cntp_ctl(uint64_t val) {
	__asm__ volatile("msr cntp_ctl_el0, %0" :: "r"(val));
}

static uint64_t ticks_per_ms;

void timer_init(void) {
	// GIC initialization order per IHI 0048B Section 4.4.2:
	//   1. Disable distributor
	//   2. Configure interrupt properties
	//   3. Enable distributor
	//   4. Configure CPU interface

	GICD_CTLR = 0;

	// Configure INTID 30 (EL1 Physical Timer PPI).
	// PPIs (INTIDs 16-31) have fixed routing — ITARGETSR is read-only.
	// Set priority to 0 (highest).
	GICD_IPRIORITYR[30 / 4] = 0x00000000;

	// Enable INTID 30 in ISENABLER0 (INTIDs 0-31).
	GICD_ISENABLER(0) = (1 << IRQ_TIMER_EL1_PHYS);

	// Enable distributor and CPU interface.
	GICD_CTLR = 1;
	GICC_PMR  = 0xFF;
	GICC_CTLR = 1;

	// Configure the ARM Generic Timer — ARM ARM D11.2.4
	ticks_per_ms = read_cntfrq() / 1000;
	write_cntp_cval(read_cntpct() + ticks_per_ms * TIMER_TICK_MS);
	write_cntp_ctl(1);  // ENABLE=1, IMASK=0

	console_println("Timer: ok");
}

void timer_delay_ms(uint32_t ms) {
	uint64_t target = read_cntpct() + ticks_per_ms * ms;
	while (read_cntpct() < target)
		;
}

void timer_handle_irq(void) {
	// Program the next tick. Called from the IRQ dispatcher — GICC_IAR/EOIR
	// are handled by the dispatcher, not here.
	write_cntp_cval(read_cntpct() + ticks_per_ms * TIMER_TICK_MS);
}
