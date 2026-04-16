// clear — clear the terminal screen
//
// Emits ANSI escape sequences:
//   ESC[2J  — erase entire display
//   ESC[H   — move cursor to home position (1,1)

#![no_std]
#![no_main]

use bazzulto_system::raw;

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8) -> ! {
    raw::raw_write(1, b"\x1b[2J\x1b[H".as_ptr(), 7);
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
