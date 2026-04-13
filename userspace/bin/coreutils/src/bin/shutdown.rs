#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    // Parse flags: -r / --reboot → reboot instead of power off.
    let mut do_reboot = false;
    for argument in arguments.iter().skip(1) {
        let flag = argument.as_str();
        if flag == "-r" || flag == "--reboot" {
            do_reboot = true;
        } else if flag == "-h" || flag == "--halt" || flag == "-P" || flag == "--poweroff" {
            do_reboot = false;
        } else if flag == "--help" {
            write_stderr("usage: shutdown [-r|--reboot] [-h|-P|--poweroff|--halt]\n");
            raw::raw_exit(0);
        }
        // Ignore unrecognised flags (time arguments, "now", etc.)
    }

    if do_reboot {
        write_stderr("shutdown: initiating system reboot...\n");
        raw::raw_machine_reboot()
    } else {
        write_stderr("shutdown: powering off...\n");
        raw::raw_machine_poweroff()
    }
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
