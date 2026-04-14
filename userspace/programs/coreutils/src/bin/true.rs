// POSIX.1-2024 — true
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/true.html
//
// Return with exit code zero. No arguments, no I/O, no environment variables.

#![no_std]
#![no_main]

use bazzulto_system::raw;

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8) -> ! {
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
