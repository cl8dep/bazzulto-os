// POSIX — su (switch user)
//
// Setuid-root binary (mode 4755). When exec'd by uid=1000, the kernel
// sets euid=0 via the setuid bit, allowing su to change identity.
//
// Usage: su [username]   (default: "system")
//
// Written WITHOUT alloc — only stack buffers and raw syscalls.
// The userspace heap allocator has issues in setuid binaries.

#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::environment::{Environment, SpecialFile};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, _envp: *const *const u8) -> ! {
    // Parse target username from argv (default: "system").
    let mut target_user = b"system" as &[u8];
    if argc > 1 && !argv.is_null() {
        let arg1 = unsafe { *argv.add(1) };
        if !arg1.is_null() {
            let mut len = 0;
            while unsafe { *arg1.add(len) } != 0 && len < 64 { len += 1; }
            target_user = unsafe { core::slice::from_raw_parts(arg1, len) };
        }
    }

    // Resolve uid/gid/shell from passwd.
    let mut target_uid: u32 = 0;
    let mut target_gid: u32 = 0;
    let mut shell_buf = [0u8; 128];
    let mut shell_len: usize = 0;
    if !resolve_user_from_passwd(target_user, &mut target_uid, &mut target_gid, &mut shell_buf, &mut shell_len) {
        raw::raw_write(1, b"su: unknown user\n".as_ptr(), 17);
        raw::raw_exit(1);
    }

    // Password check: only when escalating to root from non-root.
    if target_uid == 0 && raw::raw_getuid() != 0 {
        // Read expected hash from shadow.
        let mut expected_hash = [0u8; 128];
        let mut hash_len: usize = 0;
        if !read_shadow_hash(target_user, &mut expected_hash, &mut hash_len) {
            raw::raw_write(1, b"su: authentication failure\n".as_ptr(), 27);
            raw::raw_exit(1);
        }
        // Locked account (* or !)
        if hash_len == 1 && (expected_hash[0] == b'*' || expected_hash[0] == b'!') {
            raw::raw_write(1, b"su: account locked\n".as_ptr(), 19);
            raw::raw_exit(1);
        }

        raw::raw_write(1, b"Password: ".as_ptr(), 10);
        let mut input = [0u8; 256];
        let n = raw::raw_read(0, input.as_mut_ptr(), input.len());
        if n <= 0 {
            raw::raw_write(1, b"\nsu: authentication failure\n".as_ptr(), 28);
            raw::raw_exit(1);
        }
        // Trim trailing newline.
        let input_len = if n > 0 && input[(n - 1) as usize] == b'\n' { (n - 1) as usize } else { n as usize };

        if input_len != hash_len || input[..input_len] != expected_hash[..hash_len] {
            raw::raw_write(1, b"\nsu: authentication failure\n".as_ptr(), 28);
            raw::raw_exit(1);
        }
    }

    // Drop privileges to target user.
    raw::raw_setgid(target_gid);
    raw::raw_setuid(target_uid);

    // Exec the target shell.
    let argv_exec: [*const u8; 1] = [core::ptr::null()];
    let envp_exec: [*const u8; 1] = [core::ptr::null()];
    raw::raw_exec(shell_buf.as_ptr(), argv_exec.as_ptr(), envp_exec.as_ptr());

    raw::raw_write(1, b"su: exec failed\n".as_ptr(), 16);
    raw::raw_exit(1)
}

/// Read /system/config/passwd and find the line matching `username`.
/// Fills uid, gid, and shell path. Returns false if not found.
fn resolve_user_from_passwd(
    username: &[u8],
    uid: &mut u32,
    gid: &mut u32,
    shell: &mut [u8; 128],
    shell_len: &mut usize,
) -> bool {
    let path = Environment::get_special_file(SpecialFile::Passwd);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);

    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return false; }
    let mut buf = [0u8; 1024];
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n <= 0 { return false; }

    // Parse: name:x:uid:gid:gecos:home:shell
    let content = &buf[..n as usize];
    let mut start = 0;
    while start < content.len() {
        let end = content[start..].iter().position(|&b| b == b'\n')
            .map(|p| start + p).unwrap_or(content.len());
        let line = &content[start..end];
        start = end + 1;

        // Find colons.
        let mut colons = [0usize; 6];
        let mut cc = 0;
        for (i, &b) in line.iter().enumerate() {
            if b == b':' && cc < 6 { colons[cc] = i; cc += 1; }
        }
        if cc < 6 { continue; }

        let name = &line[..colons[0]];
        if name != username { continue; }

        // Parse uid (between colon[1] and colon[2]).
        *uid = parse_u32(&line[colons[1]+1..colons[2]]);
        // Parse gid (between colon[2] and colon[3]).
        *gid = parse_u32(&line[colons[2]+1..colons[3]]);
        // Shell (after colon[5]).
        let sh = &line[colons[5]+1..];
        let slen = sh.len().min(127);
        shell[..slen].copy_from_slice(&sh[..slen]);
        *shell_len = slen;
        return true;
    }
    false
}

/// Read /system/config/shadow and find the hash for `username`.
fn read_shadow_hash(username: &[u8], hash: &mut [u8; 128], hash_len: &mut usize) -> bool {
    let path = Environment::get_special_file(SpecialFile::Shadow);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);

    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return false; }
    let mut buf = [0u8; 1024];
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n <= 0 { return false; }

    let content = &buf[..n as usize];
    let mut start = 0;
    while start < content.len() {
        let end = content[start..].iter().position(|&b| b == b'\n')
            .map(|p| start + p).unwrap_or(content.len());
        let line = &content[start..end];
        start = end + 1;

        // name:hash:...
        let colon1 = match line.iter().position(|&b| b == b':') {
            Some(p) => p,
            None => continue,
        };
        let name = &line[..colon1];
        if name != username { continue; }

        let rest = &line[colon1+1..];
        let colon2 = rest.iter().position(|&b| b == b':').unwrap_or(rest.len());
        let h = &rest[..colon2];
        let hlen = h.len().min(127);
        hash[..hlen].copy_from_slice(&h[..hlen]);
        *hash_len = hlen;
        return true;
    }
    false
}

fn parse_u32(bytes: &[u8]) -> u32 {
    let mut val: u32 = 0;
    for &b in bytes {
        if b >= b'0' && b <= b'9' { val = val * 10 + (b - b'0') as u32; }
    }
    val
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
