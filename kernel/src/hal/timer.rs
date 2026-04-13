// hal/timer.rs — Timer HAL. Delegates to platform/qemu_virt/timer.rs.

/// Timer tick interval in milliseconds.
pub const TICK_MS: u32 = 10;

pub fn init() {
    // Timer is initialised as part of platform_init in the qemu_virt module.
    // This function exists for completeness of the HAL interface.
}

pub fn handle_irq() {
    unsafe { crate::platform::qemu_virt::timer_handle_irq() };
}

pub fn delay_ms(milliseconds: u64) {
    unsafe { crate::platform::qemu_virt::timer_delay_ms(milliseconds) };
}

pub fn read_counter() -> u64 {
    let count: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntpct_el0", out(reg) count, options(nostack, nomem));
    }
    count
}

pub fn read_frequency() -> u64 {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nostack, nomem));
    }
    freq
}
