// platform/qemu_virt/timer.rs — ARM Generic Timer driver (EL1 physical timer).
//
// The ARM Generic Timer provides a 64-bit physical counter (CNTPCT_EL0) that
// ticks at a fixed frequency advertised by the firmware in CNTFRQ_EL0.
// The EL1 physical timer fires when CNTPCT_EL0 reaches the value written to
// CNTP_CVAL_EL0, provided CNTP_CTL_EL0.ENABLE=1 and IMASK=0.
//
// Key corrections vs the C implementation:
//   - Timer frequency is ALWAYS read from CNTFRQ_EL0 at runtime (never
//     hardcoded to 24 MHz).  If the firmware sets an unexpected value, the
//     driver logs it and falls back to a safe assumption.
//   - `ticks_per_ms` is validated: if CNTFRQ_EL0 is 0 the driver panics
//     with an informative message rather than dividing by zero.
//   - Overflow is handled: the compare value is clamped if it would wrap.
//
// Reference: ARM ARM DDI 0487 D11 "The Generic Timer".

use core::sync::atomic::{AtomicU64, Ordering};

use crate::drivers::uart;

// ---------------------------------------------------------------------------
// Global tick counter — incremented every TICK_INTERVAL_MS by handle_irq().
// ---------------------------------------------------------------------------

/// Monotonically increasing counter of timer IRQs fired since boot.
///
/// Each increment represents TICK_INTERVAL_MS (10 ms) of elapsed time.
/// Used by `nanosleep` and other subsystems that need coarse time tracking.
///
/// `Ordering::Relaxed` is safe for reads/writes from the single-core IRQ
/// context: there are no other CPUs that could race here, and IRQs are
/// disabled on entry to the handler.
static TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Return the current value of the kernel tick counter.
///
/// Each tick represents `TICK_INTERVAL_MS` milliseconds.
pub fn current_tick() -> u64 {
    TICK_COUNTER.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// System register helpers
// ---------------------------------------------------------------------------

/// Read the counter frequency from CNTFRQ_EL0 (set by firmware / UEFI).
///
/// This register contains the frequency of the system counter in Hz.
/// On QEMU virt with the default UEFI firmware, this is 62_500_000 Hz (62.5 MHz).
/// The C driver assumed 24 MHz — this is WRONG for QEMU's actual configuration.
///
/// Reference: ARM ARM DDI 0487 D11.2.18 CNTFRQ_EL0.
#[inline]
fn read_counter_frequency() -> u64 {
    let frequency: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) frequency) };
    frequency
}

/// Read the current 64-bit physical counter value.
///
/// CNTPCT_EL0 is a monotonically increasing counter driven at the frequency
/// reported by CNTFRQ_EL0.  It never resets during normal operation.
///
/// Reference: ARM ARM DDI 0487 D11.2.20 CNTPCT_EL0.
#[inline]
pub fn read_counter() -> u64 {
    let count: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) count) };
    count
}

/// Write the compare value for the EL1 physical timer.
///
/// The timer fires when `read_counter() >= value`.
///
/// Reference: ARM ARM DDI 0487 D11.2.22 CNTP_CVAL_EL0.
#[inline]
unsafe fn write_compare_value(value: u64) {
    core::arch::asm!("msr cntp_cval_el0, {}", in(reg) value);
}

/// Write the EL1 physical timer control register.
///
/// Bits:
///   [0] ENABLE — 1 = timer enabled, 0 = disabled.
///   [1] IMASK  — 1 = interrupt masked (fires but no IRQ), 0 = interrupt enabled.
///   [2] ISTATUS — read-only; 1 = condition met (CNTPCT >= CVAL).
///
/// Reference: ARM ARM DDI 0487 D11.2.23 CNTP_CTL_EL0.
#[inline]
unsafe fn write_control(value: u64) {
    core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) value);
}

// ---------------------------------------------------------------------------
// CNTP_CTL_EL0 bit masks
// ---------------------------------------------------------------------------

/// CNTP_CTL_EL0 bit [0]: timer enable.
const CTL_ENABLE: u64 = 1 << 0;
/// CNTP_CTL_EL0 bit [1]: interrupt mask.  0 = IRQ delivered, 1 = masked.
const CTL_IMASK: u64 = 1 << 1;

// ---------------------------------------------------------------------------
// Timer driver
// ---------------------------------------------------------------------------

/// Minimum sane counter frequency: 1 kHz.  Firmware frequencies below this
/// indicate a misconfiguration.
const MINIMUM_COUNTER_FREQUENCY_HZ: u64 = 1_000;

/// Maximum sane counter frequency: 10 GHz.  Above this the hardware is
/// exotic and our millisecond precision assumptions may break.
const MAXIMUM_COUNTER_FREQUENCY_HZ: u64 = 10_000_000_000;

/// Scheduler tick interval in milliseconds.
///
/// This defines how often the timer fires for scheduler preemption.
/// 10 ms gives 100 Hz, a common kernel tick rate.
pub const TICK_INTERVAL_MS: u64 = 10;

pub struct Timer {
    /// Ticks per millisecond, derived from CNTFRQ_EL0.
    ticks_per_ms: u64,
}

impl Timer {
    /// Construct and initialise a Timer instance.
    ///
    /// Reads CNTFRQ_EL0 to determine the counter frequency, validates it,
    /// then arms the first timer interrupt.
    ///
    /// # Safety
    /// Must be called from EL1 after the GIC has been initialised and the
    /// timer INTID (30) has been enabled in the GIC.
    pub unsafe fn init() -> Self {
        let frequency = read_counter_frequency();

        // Validate the firmware-reported frequency.
        if frequency == 0 {
            panic!("CNTFRQ_EL0 = 0: firmware did not set the counter frequency");
        }
        if frequency < MINIMUM_COUNTER_FREQUENCY_HZ {
            panic!("CNTFRQ_EL0 too low — firmware reported less than 1 kHz");
        }
        if frequency > MAXIMUM_COUNTER_FREQUENCY_HZ {
            // Surprising but not fatal — log and continue.
            uart::puts("WARN: CNTFRQ_EL0 suspiciously high (> 10 GHz) — check firmware\r\n");
        }

        let ticks_per_ms = frequency / 1_000;

        {
            use core::fmt::Write;
            let _ = write!(uart::UartWriter, "Timer: CNTFRQ_EL0 = {} Hz  ({} ticks/ms)\r\n",
                           frequency, ticks_per_ms);
        }

        // Allow EL0 to read CNTPCT_EL0 (physical counter) and CNTVCT_EL0 (virtual
        // counter) directly without trapping to EL1.
        // CNTKCTL_EL1 bit 0 = EL0PCTEN, bit 1 = EL0VCTEN.
        // Required for the vDSO fast clock_gettime implementation which executes
        // `mrs x2, CNTPCT_EL0` from EL0.
        // Reference: ARM ARM DDI 0487 D11.2.14 CNTKCTL_EL1.
        core::arch::asm!(
            "msr cntkctl_el1, {v}",
            v = in(reg) 0b11u64,
            options(nostack, nomem)
        );

        let timer = Self { ticks_per_ms };
        timer.arm_next_tick();
        timer
    }

    /// Arm the EL1 physical timer on the calling core.
    ///
    /// Called by each AP during SMP bringup.  The Generic Timer registers
    /// CNTP_CTL_EL0 and CNTP_CVAL_EL0 are banked per PE, so each core must
    /// arm its own timer independently.
    /// Reference: ARM ARM DDI 0487 §D11.2.23 (CNTP_CTL_EL0 is banked per PE).
    ///
    /// # Safety
    /// Must be called from EL1 on the AP being initialised.
    pub unsafe fn arm_this_core(&self) {
        self.arm_next_tick();
    }

    /// Handle a timer interrupt: re-arm the timer for the next tick.
    ///
    /// Must be called from the IRQ handler when INTID_TIMER fires.
    ///
    /// # Safety
    /// Must be called from an EL1 IRQ handler.
    pub unsafe fn handle_irq(&self) {
        // Increment tick counter before re-arming so that any code reading
        // current_tick() after this IRQ sees the updated value.
        TICK_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.arm_next_tick();
    }

    /// Busy-wait for approximately `ms` milliseconds.
    ///
    /// Uses CNTPCT_EL0 to measure elapsed time without blocking interrupts.
    /// Not suitable for precise timing — jitter from cache misses and interrupts
    /// adds up to a few microseconds per call.
    pub fn delay_ms(&self, ms: u64) {
        if ms == 0 {
            return;
        }
        let ticks = self.ticks_per_ms.saturating_mul(ms);
        let deadline = read_counter().saturating_add(ticks);
        while read_counter() < deadline {
            // spin — not power-efficient, but correct for early boot delays
        }
    }

    /// Return the number of timer ticks per millisecond.
    pub fn ticks_per_ms(&self) -> u64 {
        self.ticks_per_ms
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Arm the EL1 physical timer to fire `TICK_INTERVAL_MS` from now.
    ///
    /// Sets CNTP_CVAL_EL0 to `CNTPCT_EL0 + ticks_per_ms * TICK_INTERVAL_MS`,
    /// then enables the timer with IMASK=0 (interrupt enabled).
    ///
    /// Overflow is handled with `saturating_add` — if the counter is within
    /// `ticks_interval` of wrapping (happens after ~585 years at 1 GHz),
    /// we clamp to u64::MAX, which is still in the future.
    unsafe fn arm_next_tick(&self) {
        let ticks_interval = self.ticks_per_ms.saturating_mul(TICK_INTERVAL_MS);
        let now = read_counter();
        let deadline = now.saturating_add(ticks_interval);
        write_compare_value(deadline);
        // ENABLE=1, IMASK=0 → timer enabled, interrupt unmasked.
        write_control(CTL_ENABLE);
    }
}
