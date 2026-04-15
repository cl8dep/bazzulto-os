// POSIX — su (switch user)
//
// Bazzulto compatibility shim.  Exists for POSIX compliance; the normal
// administration model uses the Binary Permission Model (BPM), not su.
//
// This binary MUST be installed setuid-root (mode 4755, owner uid=0).
// When uid=1000 executes it, the kernel's setuid-on-exec sets euid=0,
// allowing su to call setuid/setgid to the target user.
//
// Usage: su [username]   (default: "system" = uid 0)
//
// Reference: docs/Roadmap.md M3 §3.9.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::string::String;
use bazzulto_system::raw;
use bazzulto_system::environment::{Environment, SpecialFile};
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);

    let arguments = args();
    let target_user = if arguments.len() > 1 {
        arguments[1].as_str()
    } else {
        "system"
    };

    // Resolve target uid/gid from passwd.
    let (target_uid, target_gid, target_shell) = match resolve_user(target_user) {
        Some(info) => info,
        None => {
            write_stderr("su: unknown user '");
            write_stderr(target_user);
            write_stderr("'\n");
            raw::raw_exit(1);
            unreachable!()
        }
    };

    // The binary must be setuid-root.  After exec, euid should be 0.
    let current_euid = raw::raw_geteuid();
    if current_euid != 0 {
        write_stderr("su: must be setuid root\n");
        raw::raw_exit(1);
    }

    // Password check: required when a non-root user escalates to root.
    if target_uid == 0 && raw::raw_getuid() != 0 {
        if !verify_password(target_user) {
            write_stderr("su: authentication failure\n");
            raw::raw_exit(1);
        }
    }

    // Drop privileges to target user.
    raw::raw_setgid(target_gid);
    raw::raw_setuid(target_uid);

    // Exec the target user's shell with proper argv and envp.
    let mut shell_buf = [0u8; 256];
    let shell_len = target_shell.len().min(255);
    shell_buf[..shell_len].copy_from_slice(&target_shell.as_bytes()[..shell_len]);

    // argv: [shell_path, NULL]
    let argv: [*const u8; 2] = [shell_buf.as_ptr(), core::ptr::null()];

    // envp: inherit basic environment variables.
    let home_var = b"HOME=/home/user\0";
    let path_var = b"PATH=/system/bin\0";
    let term_var = b"TERM=vt100\0";
    let envp: [*const u8; 4] = [
        home_var.as_ptr(),
        path_var.as_ptr(),
        term_var.as_ptr(),
        core::ptr::null(),
    ];
    raw::raw_exec(shell_buf.as_ptr(), argv.as_ptr(), envp.as_ptr());

    write_stderr("su: failed to exec shell\n");
    raw::raw_exit(1)
}

fn resolve_user(username: &str) -> Option<(u32, u32, String)> {
    let content = read_special_file(SpecialFile::Passwd)?;
    for line in content.lines() {
        let mut fields = line.splitn(8, ':');
        let name    = fields.next()?;
        if name != username { continue; }
        let _pass   = fields.next();
        let uid_str = fields.next()?;
        let gid_str = fields.next()?;
        let _gecos  = fields.next();
        let _home   = fields.next();
        let shell   = fields.next().unwrap_or("/system/bin/sh");
        let uid: u32 = uid_str.parse().ok()?;
        let gid: u32 = gid_str.parse().ok()?;
        return Some((uid, gid, String::from(shell)));
    }
    None
}

fn verify_password(username: &str) -> bool {
    let content = match read_special_file(SpecialFile::Shadow) {
        Some(c) => c,
        None => return false,
    };
    for line in content.lines() {
        let mut fields = line.splitn(3, ':');
        let name = match fields.next() { Some(n) => n, None => continue };
        if name != username { continue; }
        let hash = match fields.next() { Some(h) => h, None => continue };
        if hash == "*" || hash == "!" || hash.is_empty() {
            return false; // Account locked
        }
        // v1.0: plaintext comparison.
        write_stdout("Password: ");
        let input = read_line_stdin();
        return input.trim() == hash;
    }
    false
}

fn read_special_file(file: SpecialFile) -> Option<String> {
    let path = Environment::get_special_file(file);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return None; }
    let mut buf = [0u8; 2048];
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n <= 0 { return None; }
    core::str::from_utf8(&buf[..n as usize]).ok().map(String::from)
}

fn read_line_stdin() -> String {
    let mut buf = [0u8; 256];
    let n = raw::raw_read(0, buf.as_mut_ptr(), buf.len());
    if n <= 0 { return String::new(); }
    String::from(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""))
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
