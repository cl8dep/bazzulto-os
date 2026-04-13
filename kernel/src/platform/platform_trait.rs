// platform/platform_trait.rs — Hardware platform abstraction trait.
//
// Provides a uniform interface for accessing platform-specific hardware
// parameters regardless of the underlying SoC or machine configuration.

use core::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// Platform trait
// ---------------------------------------------------------------------------

/// Abstract description of the hardware platform detected at boot.
pub trait Platform: Send + Sync {
    /// Physical base address of the PL011-compatible UART.
    fn uart_phys_base(&self) -> u64;
    /// Physical base address of the GIC Distributor (GICD).
    fn gicd_phys_base(&self) -> u64;
    /// Physical base address of the GICv2 CPU Interface (GICC). 0 for GICv3.
    fn gicc_phys_base(&self) -> u64;
    /// Physical base address of the GICv3 Redistributor (GICR). 0 for GICv2.
    fn gicr_phys_base(&self) -> u64;
    /// GIC architecture version: 2 or 3.
    fn gic_version(&self) -> u8;
    /// ARM Generic Timer EL1 physical timer PPI INTID (architectural: always 30).
    fn timer_intid(&self) -> u32;
    /// Total installed RAM in bytes.
    fn total_memory_bytes(&self) -> u64;
    /// Number of CPU cores.
    fn cpu_count(&self) -> u32;
}

// ---------------------------------------------------------------------------
// DetectedPlatform — concrete implementation built from DTB or defaults
// ---------------------------------------------------------------------------

/// A platform description built from a parsed DTB or from hardcoded defaults.
pub struct DetectedPlatform {
    pub uart_phys_base: u64,
    pub gicd_phys_base: u64,
    pub gicc_phys_base: u64,
    pub gicr_phys_base: u64,
    pub gic_version: u8,
    pub timer_intid: u32,
    pub total_memory_bytes: u64,
    pub cpu_count: u32,
}

impl Platform for DetectedPlatform {
    fn uart_phys_base(&self)      -> u64 { self.uart_phys_base }
    fn gicd_phys_base(&self)      -> u64 { self.gicd_phys_base }
    fn gicc_phys_base(&self)      -> u64 { self.gicc_phys_base }
    fn gicr_phys_base(&self)      -> u64 { self.gicr_phys_base }
    fn gic_version(&self)         -> u8  { self.gic_version    }
    fn timer_intid(&self)         -> u32 { self.timer_intid    }
    fn total_memory_bytes(&self)  -> u64 { self.total_memory_bytes }
    fn cpu_count(&self)           -> u32 { self.cpu_count      }
}

// ---------------------------------------------------------------------------
// Global detected platform — set once during boot
// ---------------------------------------------------------------------------

struct SyncCell<T>(UnsafeCell<T>);

// SAFETY: Single-core kernel; the platform is set once during boot before
// any concurrent access is possible.  SMP will require a spinlock here.
unsafe impl<T> Sync for SyncCell<T> {}

static DETECTED_PLATFORM: SyncCell<Option<DetectedPlatform>> =
    SyncCell(UnsafeCell::new(None));

/// Store the detected platform description.
///
/// Must be called exactly once during boot, from a single-threaded EL1 context,
/// before any call to `platform_get`.
///
/// # Safety
/// Must be called from a single-threaded context before IRQs are unmasked.
pub unsafe fn platform_set(info: DetectedPlatform) {
    *DETECTED_PLATFORM.0.get() = Some(info);
}

/// Retrieve a reference to the detected platform description.
///
/// # Panics
/// Panics if called before `platform_set`.
pub fn platform_get() -> &'static DetectedPlatform {
    unsafe {
        (*DETECTED_PLATFORM.0.get())
            .as_ref()
            .expect("platform_get called before platform_set")
    }
}
