// hello — simple test binary for BPM tier testing.
//
// This binary has NO .bazzulto_permissions section and is placed in
// /home/user/ (not /system/bin/), so it hits Tier 4 (unknown binary).
// permissiond should prompt the user: "Run with inherited permissions? [s/N]"

#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    raw::raw_write(1, b"Hello from an unknown binary!\n".as_ptr(), 30);
    raw::raw_write(1, b"If you see this, BPM allowed execution.\n".as_ptr(), 40);
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
