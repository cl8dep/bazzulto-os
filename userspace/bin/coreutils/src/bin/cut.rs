#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, lines_from_bytes, write_stdout, die};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut delimiter = '\t';
    let mut field: usize = 1;
    let mut path: Option<&str> = None;
    let mut argument_index = 1;

    while argument_index < arguments.len() {
        match arguments[argument_index].as_str() {
            "-d" => {
                argument_index += 1;
                if let Some(d) = arguments.get(argument_index) {
                    delimiter = d.chars().next().unwrap_or('\t');
                }
            }
            "-f" => {
                argument_index += 1;
                if let Some(f_str) = arguments.get(argument_index) {
                    field = f_str.parse::<usize>().unwrap_or(1);
                }
            }
            other => path = Some(other),
        }
        argument_index += 1;
    }

    if field == 0 {
        die("cut: fields are numbered from 1");
    }

    let data = read_file_or_stdin(path).unwrap_or_default();
    let lines = lines_from_bytes(&data);
    for line in &lines {
        let column: Option<&str> = line.split(delimiter).nth(field - 1);
        write_stdout(column.unwrap_or(""));
        write_stdout("\n");
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
