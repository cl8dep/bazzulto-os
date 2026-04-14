#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, die};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    if arguments.len() < 2 {
        die("usage: sleep <seconds>");
    }
    let seconds: u64 = arguments[1].parse::<u64>().unwrap_or(0);
    // nanosleep: the kernel expects a pointer to two u64s: [seconds, nanoseconds].
    let timespec = [seconds, 0u64];
    raw::raw_nanosleep(timespec.as_ptr());
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
