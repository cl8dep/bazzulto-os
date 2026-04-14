#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, lines_from_bytes, write_stdout};
use alloc::string::String;
use alloc::vec::Vec;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut reverse = false;
    let mut path: Option<&str> = None;

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-r" => reverse = true,
            other => path = Some(other),
        }
    }

    let data = read_file_or_stdin(path).unwrap_or_default();
    let lines = lines_from_bytes(&data);
    let mut owned: Vec<String> = lines.iter().map(|s| {
        let mut o = String::new();
        o.push_str(s);
        o
    }).collect();

    owned.sort_unstable();
    if reverse {
        owned.reverse();
    }
    for line in &owned {
        write_stdout(line);
        write_stdout("\n");
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
