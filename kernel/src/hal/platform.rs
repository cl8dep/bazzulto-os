// hal/platform.rs — Platform-level HAL: MMIO regions and init.

use super::MmioRegion;

/// Return the list of MMIO regions that must be mapped in the kernel page table.
pub fn mmio_regions() -> &'static [MmioRegion] {
    crate::platform::qemu_virt::platform_mmio_regions()
}

/// Initialise all platform hardware (GIC, timer, exceptions, virtio bus).
///
/// # Safety
/// Must be called from a single-threaded EL1 context after memory init.
pub unsafe fn init(hhdm_offset: u64) {
    crate::platform::qemu_virt::platform_init(hhdm_offset, 0);
}
