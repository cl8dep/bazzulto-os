#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, write_stdout, write_u64};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut count_lines = false;
    let mut count_words = false;
    let mut count_bytes = false;
    let mut paths: alloc::vec::Vec<&str> = alloc::vec::Vec::new();

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-l" => count_lines = true,
            "-w" => count_words = true,
            "-c" => count_bytes = true,
            other => paths.push(other),
        }
    }

    // Default: count all three.
    if !count_lines && !count_words && !count_bytes {
        count_lines = true;
        count_words = true;
        count_bytes = true;
    }

    let path = paths.first().copied();
    let data = read_file_or_stdin(path).unwrap_or_default();

    if count_lines {
        let lines = data.iter().filter(|&&b| b == b'\n').count();
        write_u64(lines as u64);
        write_stdout(" ");
    }
    if count_words {
        let text = core::str::from_utf8(&data).unwrap_or("");
        let words = text.split_whitespace().count();
        write_u64(words as u64);
        write_stdout(" ");
    }
    if count_bytes {
        write_u64(data.len() as u64);
        write_stdout(" ");
    }
    if let Some(p) = path {
        write_stdout(p);
    }
    write_stdout("\n");
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
