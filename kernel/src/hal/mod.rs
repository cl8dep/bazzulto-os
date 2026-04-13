// hal/mod.rs — Hardware Abstraction Layer for Bazzulto OS.
//
// Static dispatch: each function delegates to the platform implementation
// in platform/qemu_virt/<module>. No traits, no vtables, zero cost.

/// MMIO region descriptor for kernel page table mapping.
pub struct MmioRegion {
    pub base: u64,
    pub size: u64,
}

pub mod disk;
pub mod irq;
pub mod keyboard;
pub mod platform;
pub mod timer;
pub mod uart;
