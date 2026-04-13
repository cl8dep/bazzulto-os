// hal/keyboard.rs — Keyboard HAL. Delegates to platform/qemu_virt/keyboard_virtio.rs.

pub fn init(hhdm_offset: u64) {
    unsafe { crate::platform::qemu_virt::keyboard_virtio::keyboard_init(hhdm_offset) };
}

pub fn irq_handler() {
    unsafe { crate::platform::qemu_virt::keyboard_virtio::keyboard_irq_handler() };
}

pub fn get_irq_id() -> u32 {
    crate::platform::qemu_virt::keyboard_virtio::keyboard_get_irq_id()
}
