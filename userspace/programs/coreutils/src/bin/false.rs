// POSIX.1-2024 — false
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/false.html
//
// Return with a non-zero exit code. Exit status shall always be between 1 and 125 inclusive.

#![no_std]
#![no_main]

use bazzulto_system::raw;

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8) -> ! {
    raw::raw_exit(1)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
