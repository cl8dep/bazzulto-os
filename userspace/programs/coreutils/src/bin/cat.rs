// POSIX.1-2024 — cat
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/cat.html
//
// Read files in sequence and write their contents to standard output.
// Supports: -u (unbuffered output — no-op here; all kernel I/O is immediate)
//           '-' as a file operand meaning standard input
// Exit 0 if all files output successfully; >0 if any error occurred.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, open_file, read_fd_to_end, read_stdin_to_end, write_bytes_stdout, write_stderr};

fn cat_one(path: &str) -> bool {
    if path == "-" {
        let data = read_stdin_to_end();
        write_bytes_stdout(&data);
        return true;
    }

    match open_file(path) {
        Ok(fd) => {
            let data = read_fd_to_end(fd);
            raw::raw_close(fd);
            write_bytes_stdout(&data);
            true
        }
        Err(errno) => {
            write_stderr("cat: ");
            write_stderr(path);
            write_stderr(": ");
            write_stderr(coreutils::strerror(errno));
            write_stderr("\n");
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    // Collect non-option arguments; -u is accepted and silently ignored
    // (kernel I/O is already unbuffered).
    let mut file_operands: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    let mut end_of_options = false;
    for argument in arguments[1..].iter() {
        if end_of_options {
            file_operands.push(argument.as_str());
        } else if argument == "--" {
            end_of_options = true;
        } else if argument == "-u" {
            // -u: unbuffered output — accepted, no action needed.
        } else {
            file_operands.push(argument.as_str());
        }
    }

    let mut all_succeeded = true;

    if file_operands.is_empty() {
        let data = read_stdin_to_end();
        write_bytes_stdout(&data);
    } else {
        for path in &file_operands {
            if !cat_one(path) {
                all_succeeded = false;
            }
        }
    }

    raw::raw_exit(if all_succeeded { 0 } else { 1 })
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
