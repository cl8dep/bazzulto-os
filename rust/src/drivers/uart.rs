// PL011 UART driver — ARM DDI 0183G
//
// QEMU virt maps the PL011 at physical 0x09000000.
// We access it through the HHDM: hhdm_offset + 0x09000000.
// hhdm_offset is set by kernel_main before any UART call.
//
// This is the early-output UART used before the full HAL is up.
// It does not handle IRQs — those belong to the full hal_uart driver.

use core::ptr::{read_volatile, write_volatile};

// PL011 register offsets — DDI 0183G Section 3.3
const UARTDR_OFFSET:    usize = 0x000;
const UARTFR_OFFSET:    usize = 0x018;
const UARTIBRD_OFFSET:  usize = 0x024;
const UARTFBRD_OFFSET:  usize = 0x028;
const UARTLCR_H_OFFSET: usize = 0x02C;
const UARTCR_OFFSET:    usize = 0x030;
const UARTICR_OFFSET:   usize = 0x044;

// UARTFR bits — DDI 0183G Section 3.3.3
const FR_TXFF: u32 = 1 << 5; // TX FIFO full
const FR_BUSY: u32 = 1 << 3; // UART busy

// UARTLCR_H bits — DDI 0183G Section 3.3.7
const LCR_WLEN_8BIT: u32 = 0b11 << 5;
const LCR_FEN: u32 = 1 << 4; // FIFO enable

// UARTCR bits — DDI 0183G Section 3.3.8
const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

// ---------------------------------------------------------------------------
// Global UART base address (set once by kernel_main after HHDM is known)
// ---------------------------------------------------------------------------

static mut UART_BASE: usize = 0;

pub fn early_init(hhdm_offset: u64) {
    // Safety: called once from kernel_main before any UART use, single-core.
    unsafe {
        UART_BASE = (hhdm_offset + 0x09000000) as usize;
        uart_hw_init();
    }
}

// ---------------------------------------------------------------------------
// Internal register helpers
// ---------------------------------------------------------------------------

#[inline]
unsafe fn reg_read(offset: usize) -> u32 {
    read_volatile((UART_BASE + offset) as *const u32)
}

#[inline]
unsafe fn reg_write(offset: usize, value: u32) {
    write_volatile((UART_BASE + offset) as *mut u32, value);
}

unsafe fn uart_hw_init() {
    // Disable UART before reconfiguration — DDI 0183G Section 3.3.8.
    reg_write(UARTCR_OFFSET, 0);

    // Wait until any in-flight transmission completes.
    while reg_read(UARTFR_OFFSET) & FR_BUSY != 0 {}

    // Baud 115200 with 24 MHz reference clock.
    // Divisor = 24_000_000 / (16 * 115_200) = 13.020...
    // Integer part = 13, fractional part = 0.020... * 64 ≈ 1.
    reg_write(UARTIBRD_OFFSET, 13);
    reg_write(UARTFBRD_OFFSET, 1);

    // 8N1, FIFOs enabled.
    reg_write(UARTLCR_H_OFFSET, LCR_WLEN_8BIT | LCR_FEN);

    // Clear all pending interrupts.
    reg_write(UARTICR_OFFSET, 0x7FF);

    // Enable UART, TX, RX.
    reg_write(UARTCR_OFFSET, CR_UARTEN | CR_TXE | CR_RXE);
}

// ---------------------------------------------------------------------------
// Public output API
// ---------------------------------------------------------------------------

pub fn putc(c: u8) {
    unsafe {
        // Spin until TX FIFO has space.
        while reg_read(UARTFR_OFFSET) & FR_TXFF != 0 {}
        reg_write(UARTDR_OFFSET, c as u32);
    }
}

pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}

// ---------------------------------------------------------------------------
// fmt::Write implementation so we can use write!/writeln! macros on UART
// ---------------------------------------------------------------------------

pub struct UartWriter;

impl core::fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        puts(s);
        Ok(())
    }
}

/// Macro for formatted UART output (early debugging).
#[macro_export]
macro_rules! uart_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::drivers::uart::UartWriter, $($arg)*);
    }};
}

#[macro_export]
macro_rules! uart_println {
    () => { $crate::uart_print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::drivers::uart::UartWriter, $($arg)*);
        $crate::drivers::uart::putc(b'\r');
        $crate::drivers::uart::putc(b'\n');
    }};
}
