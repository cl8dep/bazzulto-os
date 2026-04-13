#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout};
use bazzulto_io::directory;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut long_format = false;
    let mut paths: alloc::vec::Vec<&str> = alloc::vec::Vec::new();

    for arg in &arguments[1..] {
        match arg.as_str() {
            "-l" | "-la" | "-al" => long_format = true,
            other if other.starts_with('-') => {} // ignore other flags
            other => paths.push(other),
        }
    }

    let path = paths.first().copied().unwrap_or(".");

    // Resolve relative path against cwd if needed.
    let resolved: alloc::string::String;
    let target = if path.starts_with('/') {
        path
    } else if path == "." {
        let mut cwd_buf = [0u8; 512];
        let n = raw::raw_getcwd(cwd_buf.as_mut_ptr(), cwd_buf.len());
        if n > 0 {
            // n includes the NUL terminator — strip it.
            let len = (n as usize).saturating_sub(1);
            resolved = alloc::string::String::from(
                core::str::from_utf8(&cwd_buf[..len]).unwrap_or("/")
            );
            resolved.as_str()
        } else {
            "/"
        }
    } else {
        // Prepend cwd.
        let mut cwd_buf = [0u8; 512];
        let n = raw::raw_getcwd(cwd_buf.as_mut_ptr(), cwd_buf.len());
        let cwd = if n > 0 {
            let len = (n as usize).saturating_sub(1);
            core::str::from_utf8(&cwd_buf[..len]).unwrap_or("/")
        } else {
            "/"
        };
        resolved = alloc::format!("{}/{}", cwd.trim_end_matches('/'), path);
        resolved.as_str()
    };

    let entries = directory::read_dir(target);

    if entries.is_empty() {
        // Directory might not exist or be empty — try opening to distinguish.
        let fd = raw::raw_open(target.as_ptr(), target.len());
        if fd < 0 {
            write_stdout("ls: cannot access '");
            write_stdout(target);
            write_stdout("': No such file or directory\n");
        }
        // If fd >= 0, directory is empty — print nothing.
        if fd >= 0 {
            raw::raw_close(fd as i32);
        }
        raw::raw_exit(if fd < 0 { 1 } else { 0 });
    }

    for name in &entries {
        write_stdout(name);
        if long_format {
            // Future: add size/type info here.
        }
        write_stdout("\n");
    }

    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
