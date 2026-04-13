// platform/mod.rs — Platform abstraction layer.
//
// The `qemu_virt` module is the only supported platform for now.
// Future platforms (e.g. Raspberry Pi) would add a sibling module and
// be selected via a cargo feature or build-time configuration.

pub mod dtb;
pub mod platform_trait;
pub mod qemu_virt;

pub use platform_trait::{
    DetectedPlatform, Platform, platform_get, platform_set,
};

/// Dispatch one pending IRQ from the platform interrupt controller.
///
/// Called by `exception_handler_irq_el0/el1` in `arch::arm64::exceptions`.
pub fn irq_dispatch() {
    qemu_virt::irq_dispatch();
}
