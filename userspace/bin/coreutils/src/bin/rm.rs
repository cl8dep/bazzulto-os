#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stderr, die};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    if arguments.len() < 2 {
        die("usage: rm <file>...");
    }
    let mut exit_code = 0i32;
    for path in &arguments[1..] {
        let result = raw::raw_unlink(path.as_ptr(), path.len());
        if result < 0 {
            write_stderr("rm: cannot remove '");
            write_stderr(path);
            write_stderr("'\n");
            exit_code = 1;
        }
    }
    raw::raw_exit(exit_code)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
