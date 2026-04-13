#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    // argv[0] is the program name; arguments start at index 1.
    let mut print_newline = true;
    let mut start_index = 1;
    if arguments.get(1).map(|s| s.as_str()) == Some("-n") {
        print_newline = false;
        start_index = 2;
    }
    let mut first = true;
    for argument in &arguments[start_index..] {
        if !first {
            write_stdout(" ");
        }
        write_stdout(argument);
        first = false;
    }
    if print_newline {
        write_stdout("\n");
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
