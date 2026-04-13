#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();
    if arguments.len() < 2 {
        write_stderr("mkdir: missing operand\n");
        write_stderr("usage: mkdir [-p] <directory>...\n");
        raw::raw_exit(1);
    }

    // Minimal -p flag: create parent directories as needed.
    let mut parents = false;
    let mut path_start = 1usize;
    if arguments.get(1).map(|s| s.as_str()) == Some("-p") {
        parents = true;
        path_start = 2;
    }

    let mut had_error = false;
    for directory_path in &arguments[path_start..] {
        if parents {
            make_parents(directory_path.as_str());
        } else {
            let result = raw::raw_mkdir(
                directory_path.as_ptr(),
                directory_path.len(),
                0o755,
            );
            if result < 0 {
                write_stderr("mkdir: cannot create directory: ");
                write_stderr(directory_path.as_str());
                write_stderr("\n");
                had_error = true;
            }
        }
    }

    raw::raw_exit(if had_error { 1 } else { 0 })
}

/// Create all components of `path`, ignoring EEXIST on intermediate directories.
fn make_parents(path: &str) {
    let bytes = path.as_bytes();
    let mut index = 1usize; // skip leading '/'
    loop {
        // Find next '/' or end.
        while index < bytes.len() && bytes[index] != b'/' {
            index += 1;
        }
        // Create up to this component.
        let component = match core::str::from_utf8(&bytes[..index]) {
            Ok(s) => s,
            Err(_) => break,
        };
        // Ignore EEXIST (-17) for intermediate directories.
        let _ = raw::raw_mkdir(component.as_ptr(), component.len(), 0o755);
        if index >= bytes.len() {
            break;
        }
        index += 1;
    }
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
