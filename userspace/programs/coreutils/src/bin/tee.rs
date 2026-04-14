#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();

    // Parse: tee [-a] [file...]
    let mut append = false;
    let mut file_start = 1usize;
    if arguments.get(1).map(|s| s.as_str()) == Some("-a") {
        append = true;
        file_start = 2;
    }

    // Open output files.
    let mut file_fds: [i32; 16] = [-1; 16];
    let mut file_count = 0usize;

    for file_path in &arguments[file_start..] {
        if file_count >= 16 {
            write_stderr("tee: too many files (max 16)\n");
            break;
        }
        let fd = if append {
            raw::raw_creat_append(file_path.as_ptr(), file_path.len())
        } else {
            raw::raw_creat(file_path.as_ptr(), file_path.len())
        };
        if fd < 0 {
            write_stderr("tee: cannot open: ");
            write_stderr(file_path.as_str());
            write_stderr("\n");
        } else {
            file_fds[file_count] = fd as i32;
            file_count += 1;
        }
    }

    // Read stdin, write to stdout and all files.
    let mut buffer = [0u8; 4096];
    loop {
        let bytes_read = raw::raw_read(0, buffer.as_mut_ptr(), buffer.len());
        if bytes_read <= 0 {
            break;
        }
        let count = bytes_read as usize;
        // Write to stdout.
        raw::raw_write(1, buffer.as_ptr(), count);
        // Write to each file.
        for i in 0..file_count {
            if file_fds[i] >= 0 {
                raw::raw_write(file_fds[i], buffer.as_ptr(), count);
            }
        }
    }

    // Close all output files.
    for i in 0..file_count {
        if file_fds[i] >= 0 {
            raw::raw_close(file_fds[i]);
        }
    }

    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
