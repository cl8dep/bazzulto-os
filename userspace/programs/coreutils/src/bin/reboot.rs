#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::write_stderr;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    write_stderr("reboot: initiating system reboot...\n");
    raw::raw_machine_reboot()
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
