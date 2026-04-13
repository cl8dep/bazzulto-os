// platform/qemu_virt/rtc.rs — PL031 Real Time Clock driver.
//
// The PL031 RTC is present in QEMU virt at physical address 0x09010000.
// It provides a 32-bit count of seconds since Unix epoch.  QEMU initialises
// RTCDR to the host's current Unix timestamp when the VM starts.
//
// Reference: ARM PL031 TRM DDI 0224B.
//
// Register map (offsets from base address):
//   0x000  RTCDR   — Data Register (read: current time in seconds, 32-bit)
//   0x004  RTCMR   — Match Register (for alarm interrupt, unused here)
//   0x008  RTCLR   — Load Register  (write: set current time)
//   0x00C  RTCCR   — Control Register (bit 0: enable; write 1 to enable)
//   0x010  RTCIMSC — Interrupt Mask Set/Clear
//   0x014  RTCRIS  — Raw Interrupt Status
//   0x018  RTCMIS  — Masked Interrupt Status
//   0x01C  RTCICR  — Interrupt Clear Register

// ---------------------------------------------------------------------------
// Physical base address
// ---------------------------------------------------------------------------

/// Physical base address of the PL031 RTC on QEMU virt.
///
/// Reference: QEMU hw/arm/virt.c virt_memmap[] entry for "pl031".
const PL031_PHYS_BASE: u64 = 0x09010000;

// ---------------------------------------------------------------------------
// Boot-time RTC snapshot
// ---------------------------------------------------------------------------

/// Seconds since Unix epoch recorded at boot time by reading RTCDR once.
///
/// Set exactly once by `set_boot_rtc_seconds()` during platform initialisation,
/// before any user processes run.
///
/// SAFETY: written once during single-threaded boot, then only read.
static mut BOOT_RTC_SECONDS: u64 = 0;

/// Record the boot-time wall-clock second from the PL031 RTCDR register.
///
/// Must be called exactly once during `platform_init`, before IRQs are unmasked.
///
/// # Safety
/// Must be called from a single-threaded boot context.
pub unsafe fn set_boot_rtc_seconds(seconds: u64) {
    BOOT_RTC_SECONDS = seconds;
}

/// Return the current real-time clock value as `(seconds_since_epoch, subsecond_ns)`.
///
/// The seconds component is the boot-time RTCDR snapshot plus the elapsed
/// monotonic time (derived from the kernel tick counter).  The subsecond
/// component is the millisecond remainder of the elapsed time expressed as
/// nanoseconds.
///
/// # Safety
/// Reads the `BOOT_RTC_SECONDS` static, which must have been set by
/// `set_boot_rtc_seconds` before this function is called.
pub unsafe fn realtime_now(current_tick: u64, tick_interval_ms: u64) -> (u64, u64) {
    let elapsed_ms  = current_tick.saturating_mul(tick_interval_ms);
    let elapsed_sec = elapsed_ms / 1_000;
    let remainder_ns = (elapsed_ms % 1_000).saturating_mul(1_000_000);
    (BOOT_RTC_SECONDS.saturating_add(elapsed_sec), remainder_ns)
}

// ---------------------------------------------------------------------------
// PL031 driver struct
// ---------------------------------------------------------------------------

/// Driver for the ARM PL031 Real Time Clock.
///
/// Holds the HHDM-mapped virtual base address of the device's register block.
pub struct Pl031Rtc {
    base_virt: usize,
}

impl Pl031Rtc {
    /// Construct the RTC driver.
    ///
    /// `hhdm_offset` is the HHDM direct-map offset added to the physical
    /// address to obtain the virtual address, following the same pattern used
    /// by the GIC and UART drivers.
    pub fn new(hhdm_offset: u64) -> Self {
        Self {
            base_virt: (PL031_PHYS_BASE + hhdm_offset) as usize,
        }
    }

    /// Enable the RTC by writing 1 to RTCCR bit 0.
    ///
    /// Must be called once after construction before `read_seconds()` is used.
    ///
    /// Reference: PL031 TRM DDI 0224B §3.3.4 "RTCCR register".
    pub fn enable(&self) {
        unsafe {
            core::ptr::write_volatile((self.base_virt + 0x00C) as *mut u32, 1);
        }
    }

    /// Read the current time in seconds since Unix epoch from RTCDR.
    ///
    /// QEMU initialises RTCDR to the host's current Unix timestamp when the
    /// VM starts; subsequent reads reflect elapsed seconds.
    ///
    /// Reference: PL031 TRM DDI 0224B §3.3.1 "RTCDR register".
    pub fn read_seconds(&self) -> u32 {
        unsafe {
            core::ptr::read_volatile((self.base_virt + 0x000) as *const u32)
        }
    }
}
