#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::write_stdout;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let mut buffer = [0u8; 4096];
    let result = raw::raw_getcwd(buffer.as_mut_ptr(), buffer.len());
    if result <= 0 {
        write_stdout("pwd: cannot get current directory\n");
        raw::raw_exit(1);
    }

    // result is the number of bytes written including the null terminator.
    let length = (result as usize).saturating_sub(1); // strip null terminator
    if let Ok(path) = core::str::from_utf8(&buffer[..length]) {
        write_stdout(path);
        write_stdout("\n");
    }

    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
