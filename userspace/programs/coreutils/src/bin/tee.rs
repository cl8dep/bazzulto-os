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
        let mut file_path_buf = [0u8; 512];
        let file_path_len = file_path.len().min(511);
        file_path_buf[..file_path_len].copy_from_slice(&file_path.as_bytes()[..file_path_len]);
        let fd = if append {
            // O_WRONLY|O_CREAT|O_APPEND = 1|0x40|0x400
            raw::raw_open(file_path_buf.as_ptr(), 1 | 0x40 | 0x400, 0o666)
        } else {
            raw::raw_creat(file_path_buf.as_ptr(), 0o666)
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
