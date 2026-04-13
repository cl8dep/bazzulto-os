// hal/uart.rs — UART HAL. Delegates to platform/qemu_virt/pl011.rs.

pub fn init(hhdm_offset: u64) {
    crate::drivers::uart::early_init(hhdm_offset);
}

pub fn putc(byte: u8) {
    crate::drivers::uart::putc(byte);
}

pub fn getc() -> u8 {
    // Read one byte blocking from UART — used for early debug input.
    crate::drivers::tty::uart_receive_blocking()
}

pub fn puts(string: &str) {
    crate::drivers::uart::puts(string);
}

pub fn irq_handler() {
    // PL011 UART IRQ: read characters and push to input layer.
    // For now, the TTY layer handles UART input directly.
}
