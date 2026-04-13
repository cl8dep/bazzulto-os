#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, lines_from_bytes, write_stdout, write_u64};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut show_count = false;
    let mut path: Option<&str> = None;

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-c" => show_count = true,
            other => path = Some(other),
        }
    }

    let data = read_file_or_stdin(path).unwrap_or_default();
    let lines = lines_from_bytes(&data);

    let mut previous: Option<&str> = None;
    let mut count: u64 = 0;

    for line in &lines {
        match previous {
            Some(previous_line) if previous_line == *line => {
                count += 1;
            }
            _ => {
                if let Some(previous_line) = previous {
                    if show_count {
                        write_u64(count);
                        write_stdout(" ");
                    }
                    write_stdout(previous_line);
                    write_stdout("\n");
                }
                previous = Some(line);
                count = 1;
            }
        }
    }
    if let Some(previous_line) = previous {
        if show_count {
            write_u64(count);
            write_stdout(" ");
        }
        write_stdout(previous_line);
        write_stdout("\n");
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
