// basename — return non-directory portion of a pathname
//
// POSIX.1-2024 specification:
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/basename.html
//
// Synopsis:
//   basename string [suffix]
//
// Algorithm (POSIX §steps 1–6):
//   1. Null string → unspecified; we return '.'.
//   2. "//" → implementation-defined; we process normally (steps 3–6).
//   3. All slashes → return "/".
//   4. Strip trailing slashes.
//   5. Strip up to and including the last remaining slash.
//   6. Strip suffix if present, not identical to the whole string, and is a suffix.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::string::String;
use bazzulto_system::raw;
use coreutils::{args, write_stderr, write_stdout};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    if arguments.len() < 2 {
        write_stderr("basename: missing operand\n");
        write_stderr("usage: basename string [suffix]\n");
        raw::raw_exit(1);
    }

    let input = arguments[1].as_str();
    let suffix = arguments.get(2).map(|s| s.as_str());

    let result = basename(input, suffix);
    write_stdout(&result);
    write_stdout("\n");
    raw::raw_exit(0)
}

fn basename<'a>(input: &'a str, suffix: Option<&'a str>) -> String {
    // Step 1: null string → return '.' (implementation choice).
    if input.is_empty() {
        return String::from(".");
    }

    // Step 2: "//" → implementation-defined. We process normally (fall through).

    // Step 3: string consists entirely of slashes → return "/".
    if input.bytes().all(|b| b == b'/') {
        return String::from("/");
    }

    // Step 4: remove trailing slashes.
    let trimmed = input.trim_end_matches('/');

    // Step 5: remove everything up to and including the last slash.
    let after_last_slash = match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    };

    // Step 6: remove suffix if present, not identical to the whole remaining
    // string, and is a suffix of the remaining string.
    let result = match suffix {
        Some(s) if !s.is_empty() && after_last_slash != s && after_last_slash.ends_with(s) => {
            &after_last_slash[..after_last_slash.len() - s.len()]
        }
        _ => after_last_slash,
    };

    String::from(result)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
