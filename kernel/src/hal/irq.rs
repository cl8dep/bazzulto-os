// hal/irq.rs — IRQ controller HAL. Delegates to platform/qemu_virt/gicv2.rs.

use crate::platform::qemu_virt::gicv2;

/// GIC INTID for the EL1 physical timer (PPI 14, INTID 30).
pub const TIMER_IRQ: u32 = gicv2::INTID_TIMER;

/// GIC INTID for PL011 UART0 (SPI 1, INTID 33).
pub const UART_IRQ: u32 = gicv2::INTID_UART;

/// GIC spurious interrupt identifier (no real interrupt pending).
pub const SPURIOUS: u32 = gicv2::INTID_SPURIOUS;

/// Enable interrupt `intid` in the GIC distributor.
///
/// # Safety
/// Must be called after GIC init, from EL1.
pub unsafe fn enable(intid: u32) {
    crate::platform::qemu_virt::gic_enable_interrupt(intid);
}

/// Acknowledge the highest-priority pending interrupt. Returns its INTID.
///
/// # Safety
/// Must be called from an IRQ handler.
pub unsafe fn acknowledge() -> u32 {
    crate::platform::qemu_virt::gic_acknowledge()
}

/// Signal end-of-interrupt for `intid`.
///
/// # Safety
/// Must be called after handling the interrupt.
pub unsafe fn end(intid: u32) {
    crate::platform::qemu_virt::gic_end_of_interrupt(intid);
}
