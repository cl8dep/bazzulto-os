#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, lines_from_bytes, write_stdout};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut line_count: usize = 10;
    let mut path: Option<&str> = None;
    let mut argument_index = 1;

    while argument_index < arguments.len() {
        match arguments[argument_index].as_str() {
            "-n" => {
                argument_index += 1;
                if let Some(n_str) = arguments.get(argument_index) {
                    line_count = n_str.parse::<usize>().unwrap_or(10);
                }
            }
            other => path = Some(other),
        }
        argument_index += 1;
    }

    let data = read_file_or_stdin(path).unwrap_or_default();
    let lines = lines_from_bytes(&data);
    for line in lines.iter().take(line_count) {
        write_stdout(line);
        write_stdout("\n");
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
