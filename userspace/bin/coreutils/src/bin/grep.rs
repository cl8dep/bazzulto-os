#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, lines_from_bytes, write_stdout, die};
use alloc::string::String;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut invert = false;
    let mut case_insensitive = false;
    let mut remaining: alloc::vec::Vec<&str> = alloc::vec::Vec::new();

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-v" => invert = true,
            "-i" => case_insensitive = true,
            "-vi" | "-iv" => { invert = true; case_insensitive = true; }
            other => remaining.push(other),
        }
    }

    if remaining.is_empty() {
        die("usage: grep [-v] [-i] <pattern> [file]");
    }
    let pattern = remaining[0];
    let path = remaining.get(1).copied();

    let pattern_lower: String = if case_insensitive {
        pattern.chars().flat_map(|c| c.to_lowercase()).collect()
    } else {
        String::new()
    };
    let effective_pattern = if case_insensitive { pattern_lower.as_str() } else { pattern };

    let data = read_file_or_stdin(path).unwrap_or_default();
    let lines = lines_from_bytes(&data);
    let mut matched = false;

    for line in &lines {
        let haystack: String = if case_insensitive {
            line.chars().flat_map(|c| c.to_lowercase()).collect()
        } else {
            String::new()
        };
        let effective_line = if case_insensitive { haystack.as_str() } else { line };
        let contains = effective_line.contains(effective_pattern);
        let print = if invert { !contains } else { contains };
        if print {
            write_stdout(line);
            write_stdout("\n");
            matched = true;
        }
    }

    raw::raw_exit(if matched { 0 } else { 1 })
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
