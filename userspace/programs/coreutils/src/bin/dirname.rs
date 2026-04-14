#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout, die};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    if arguments.len() < 2 {
        die("usage: dirname <path>");
    }
    let path = &arguments[1];
    let result = if let Some(slash_pos) = path.rfind('/') {
        if slash_pos == 0 {
            "/"
        } else {
            &path[..slash_pos]
        }
    } else {
        "."
    };
    write_stdout(result);
    write_stdout("\n");
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
