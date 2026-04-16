// readhome — test binary that tries to read /home/user/.
//
// Placed in /home/user/, this binary is Tier 4 (unknown).
// It tests whether inherited permissions allow reading user's home dir.

#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    raw::raw_write(1, b"readhome: attempting to list /home/user/\n".as_ptr(), 41);

    let fd = raw::raw_open(b"/home/user\0".as_ptr(), 0, 0);
    if fd < 0 {
        raw::raw_write(1, b"readhome: access denied (EACCES)\n".as_ptr(), 33);
    } else {
        raw::raw_write(1, b"readhome: access granted\n".as_ptr(), 25);
        raw::raw_close(fd as i32);
    }

    raw::raw_write(1, b"readhome: attempting to read /system/config/shadow\n".as_ptr(), 51);
    let fd2 = raw::raw_open(b"/system/config/shadow\0".as_ptr(), 0, 0);
    if fd2 < 0 {
        raw::raw_write(1, b"readhome: shadow access denied (expected)\n".as_ptr(), 43);
    } else {
        raw::raw_write(1, b"readhome: shadow access GRANTED (unexpected!)\n".as_ptr(), 47);
        raw::raw_close(fd2 as i32);
    }

    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
