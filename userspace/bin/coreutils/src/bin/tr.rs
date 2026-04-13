#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_stdin_to_end, write_bytes_stdout, die};
use alloc::vec::Vec;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    // tr [-d] SET1 [SET2]
    let mut delete_mode = false;
    let mut remaining: Vec<&str> = Vec::new();

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-d" => delete_mode = true,
            other => remaining.push(other),
        }
    }

    if remaining.is_empty() {
        die("usage: tr [-d] <set1> [<set2>]");
    }

    let set1: Vec<char> = remaining[0].chars().collect();
    let set2: Vec<char> = remaining.get(1).map(|s| s.chars().collect()).unwrap_or_default();

    let data = read_stdin_to_end();
    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => {
            // Pass-through non-UTF-8 bytes unchanged.
            write_bytes_stdout(&data);
            raw::raw_exit(0)
        }
    };

    let mut output: Vec<u8> = Vec::with_capacity(data.len());
    for character in text.chars() {
        if delete_mode {
            if !set1.contains(&character) {
                let mut encoded = [0u8; 4];
                let encoded_str = character.encode_utf8(&mut encoded);
                output.extend_from_slice(encoded_str.as_bytes());
            }
        } else {
            let translated = set1.iter().position(|&c| c == character)
                .and_then(|index| set2.get(index).copied())
                .unwrap_or(character);
            let mut encoded = [0u8; 4];
            let encoded_str = translated.encode_utf8(&mut encoded);
            output.extend_from_slice(encoded_str.as_bytes());
        }
    }
    write_bytes_stdout(&output);
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
