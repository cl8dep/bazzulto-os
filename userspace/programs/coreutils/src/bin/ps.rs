#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{write_stdout, write_stderr};

// dirent64 header size: ino(8) + off(8) + reclen(2) + type(1) = 19 bytes before name.
const DIRENT64_HEADER_SIZE: usize = 19;

// DT_DIR file type.
const DT_DIR: u8 = 4;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    // Open /proc directory.
    let proc_path = b"/proc\0";
    let proc_fd = raw::raw_open(proc_path.as_ptr(), 0, 0);
    if proc_fd < 0 {
        write_stderr("ps: cannot open /proc\n");
        raw::raw_exit(1);
    }

    write_stdout("  PID COMM\n");

    // Read directory entries, one dirent64 buffer at a time.
    let mut buf = [0u8; 4096];
    loop {
        let bytes_read = raw::raw_getdents64(proc_fd as i32, buf.as_mut_ptr(), buf.len());
        if bytes_read <= 0 {
            break;
        }

        let mut offset = 0usize;
        while offset < bytes_read as usize {
            if offset + DIRENT64_HEADER_SIZE > bytes_read as usize {
                break;
            }

            // Parse dirent64 fields.
            let record_length = u16::from_le_bytes([
                buf[offset + 16],
                buf[offset + 17],
            ]) as usize;
            let entry_type = buf[offset + 18];

            if record_length == 0 {
                break;
            }

            // Name starts at offset + 19, null-terminated.
            let name_start = offset + DIRENT64_HEADER_SIZE;
            let name_end = (offset + record_length).min(bytes_read as usize);
            let name_bytes = &buf[name_start..name_end];

            // Find null terminator.
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
            let name = match core::str::from_utf8(&name_bytes[..name_len]) {
                Ok(s) => s,
                Err(_) => { offset += record_length; continue; }
            };

            // Skip "." and ".." and non-numeric entries (non-PID entries like "self").
            let is_pid_dir = entry_type == DT_DIR && name.bytes().all(|b| b >= b'0' && b <= b'9');
            if is_pid_dir {
                print_process_info(name);
            }

            offset += record_length;
        }
    }

    raw::raw_close(proc_fd as i32);
    raw::raw_exit(0)
}

/// Read /proc/<pid>/comm and print the process line.
fn print_process_info(pid_str: &str) {
    // Pad PID to 5 chars (right-aligned).
    let pid_len = pid_str.len();
    let pad = if pid_len < 5 { 5 - pid_len } else { 0 };
    for _ in 0..pad {
        write_stdout(" ");
    }
    write_stdout(pid_str);
    write_stdout(" ");

    // Try to read /proc/<pid>/comm for the process name.
    let mut comm_path = [0u8; 32];
    let prefix = b"/proc/";
    let suffix = b"/comm";
    let mut path_len = 0usize;
    for &b in prefix { comm_path[path_len] = b; path_len += 1; }
    for b in pid_str.bytes() { comm_path[path_len] = b; path_len += 1; }
    for &b in suffix { comm_path[path_len] = b; path_len += 1; }

    let comm_fd = raw::raw_open(comm_path.as_ptr(), 0, 0);
    if comm_fd >= 0 {
        let mut comm_buf = [0u8; 256];
        let n = raw::raw_read(comm_fd as i32, comm_buf.as_mut_ptr(), comm_buf.len());
        raw::raw_close(comm_fd as i32);
        if n > 0 {
            let len = n as usize;
            // Strip trailing newline.
            let end = if comm_buf[len - 1] == b'\n' { len - 1 } else { len };
            if let Ok(name) = core::str::from_utf8(&comm_buf[..end]) {
                write_stdout(name);
            }
        }
    } else {
        write_stdout("?");
    }

    write_stdout("\n");
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
