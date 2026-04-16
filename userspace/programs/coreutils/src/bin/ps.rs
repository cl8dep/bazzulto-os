// POSIX.1-2024 — ps (report process status)
//
// Default output: PID TTY TIME CMD
// -f (full):      UID PID PPID STIME TTY TIME CMD
// -e / -A:        all processes (default: same euid only)
//
// Reads process info from /proc/<pid>/ virtual filesystem.
// Written without alloc (stack buffers only).
//
// Reference: https://pubs.opengroup.org/onlinepubs/9799919799/utilities/ps.html

#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::environment::{Environment, SpecialFile};

const DIRENT64_HEADER_SIZE: usize = 19;
const DT_DIR: u8 = 4;

struct Options {
    all: bool,     // -e, -A, -a: show all processes
    full: bool,    // -f: full listing (UID, PPID, STIME)
}

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, _envp: *const *const u8) -> ! {
    let mut opts = Options { all: false, full: false };

    // Parse options.
    for i in 1..argc {
        let arg = unsafe { *argv.add(i) };
        if arg.is_null() { break; }
        let mut len = 0;
        while unsafe { *arg.add(len) } != 0 && len < 16 { len += 1; }
        let s = unsafe { core::slice::from_raw_parts(arg, len) };
        if s == b"-e" || s == b"-A" || s == b"-a" { opts.all = true; }
        if s == b"-f" { opts.full = true; }
        if s == b"-ef" || s == b"-fe" { opts.all = true; opts.full = true; }
    }

    let my_euid = raw::raw_geteuid();

    // Load passwd for UID→name resolution (only for -f).
    let mut passwd_buf = [0u8; 1024];
    let passwd_len = if opts.full { read_file_to_buf(SpecialFile::Passwd, &mut passwd_buf) } else { 0 };

    // Print header.
    if opts.full {
        out(b"     UID   PID  PPID TTY      TIME CMD\n");
    } else {
        out(b"  PID TTY      TIME CMD\n");
    }

    // Open /proc.
    let proc_fd = raw::raw_open(b"/proc\0".as_ptr(), 0, 0);
    if proc_fd < 0 { raw::raw_exit(1); }

    let mut buf = [0u8; 4096];
    loop {
        let n = raw::raw_getdents64(proc_fd as i32, buf.as_mut_ptr(), buf.len());
        if n <= 0 { break; }

        let mut off = 0usize;
        while off < n as usize {
            if off + DIRENT64_HEADER_SIZE > n as usize { break; }
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            let dtype = buf[off + 18];
            if reclen == 0 { break; }

            let name_start = off + DIRENT64_HEADER_SIZE;
            let name_end = (off + reclen).min(n as usize);
            let name_bytes = &buf[name_start..name_end];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
            let name = &name_bytes[..name_len];

            // Only numeric directories (PIDs).
            if dtype == DT_DIR && name.iter().all(|&b| b >= b'0' && b <= b'9') {
                let pid = parse_u32(name);
                print_process(pid, name, &opts, my_euid, &passwd_buf[..passwd_len]);
            }

            off += reclen;
        }
    }
    raw::raw_close(proc_fd as i32);
    raw::raw_exit(0)
}

fn print_process(pid: u32, pid_str: &[u8], opts: &Options, my_euid: u32, passwd: &[u8]) {
    // Read /proc/<pid>/status for uid, ppid, comm.
    let mut path = [0u8; 48];
    let plen = build_proc_path(&mut path, pid_str, b"/status");
    let status_fd = raw::raw_open(path.as_ptr(), 0, 0);
    if status_fd < 0 { return; }
    let mut sbuf = [0u8; 512];
    let sn = raw::raw_read(status_fd as i32, sbuf.as_mut_ptr(), sbuf.len());
    raw::raw_close(status_fd as i32);
    if sn <= 0 { return; }
    let status = &sbuf[..sn as usize];

    // Parse fields from status.
    let proc_uid = get_status_field_u32(status, b"Uid:");
    let proc_ppid = get_status_field_u32(status, b"PPid:");
    let comm = get_status_field_str(status, b"Name:");

    // Filter: default shows only processes with same euid.
    if !opts.all && proc_uid != my_euid { return; }

    if opts.full {
        // UID (right-aligned, 8 chars) — resolve to name if possible.
        let uid_name = resolve_uid_name(proc_uid, passwd);
        pad_right(&uid_name, 8);

        // PID (right-aligned, 5 chars)
        out(b" ");
        let mut pid_buf = [0u8; 10];
        let pid_s = format_u32(pid, &mut pid_buf);
        pad_left(pid_s, 5);

        // PPID (right-aligned, 5 chars)
        out(b" ");
        let mut ppid_buf = [0u8; 10];
        let ppid_s = format_u32(proc_ppid, &mut ppid_buf);
        pad_left(ppid_s, 5);
    } else {
        // PID (right-aligned, 5 chars)
        let mut pid_buf = [0u8; 10];
        let pid_s = format_u32(pid, &mut pid_buf);
        pad_left(pid_s, 5);
    }

    // TTY
    out(b" ?        "); // No controlling terminal info yet

    // TIME (cumulative CPU time — not tracked yet, show 00:00:00)
    out(b"00:00:00 ");

    // CMD
    if comm.is_empty() {
        out(b"?");
    } else {
        raw::raw_write(1, comm.as_ptr(), comm.len());
    }
    out(b"\n");
}

// ---------------------------------------------------------------------------
// /proc/<pid>/status field parsing
// ---------------------------------------------------------------------------

fn get_status_field_u32(status: &[u8], field: &[u8]) -> u32 {
    if let Some(pos) = find_bytes(status, field) {
        let start = pos + field.len();
        // Skip whitespace.
        let mut i = start;
        while i < status.len() && (status[i] == b' ' || status[i] == b'\t') { i += 1; }
        let mut end = i;
        while end < status.len() && status[end] >= b'0' && status[end] <= b'9' { end += 1; }
        parse_u32(&status[i..end])
    } else {
        0
    }
}

fn get_status_field_str<'a>(status: &'a [u8], field: &[u8]) -> &'a [u8] {
    if let Some(pos) = find_bytes(status, field) {
        let start = pos + field.len();
        let mut i = start;
        while i < status.len() && (status[i] == b' ' || status[i] == b'\t') { i += 1; }
        let mut end = i;
        while end < status.len() && status[end] != b'\n' && status[end] != 0 { end += 1; }
        &status[i..end]
    } else {
        b""
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() { return None; }
    for i in 0..=(haystack.len() - needle.len()) {
        if &haystack[i..i + needle.len()] == needle { return Some(i); }
    }
    None
}

// ---------------------------------------------------------------------------
// UID → name resolution
// ---------------------------------------------------------------------------

fn resolve_uid_name(uid: u32, passwd: &[u8]) -> [u8; 16] {
    let mut result = [b' '; 16];
    // Search passwd for uid.
    let mut start = 0;
    while start < passwd.len() {
        let end = passwd[start..].iter().position(|&b| b == b'\n')
            .map(|p| start + p).unwrap_or(passwd.len());
        let line = &passwd[start..end];
        start = end + 1;

        // name:x:uid:...
        let mut colons = [0usize; 3];
        let mut cc = 0;
        for (i, &b) in line.iter().enumerate() {
            if b == b':' && cc < 3 { colons[cc] = i; cc += 1; }
            if cc >= 3 { break; }
        }
        if cc < 3 { continue; }
        let file_uid = parse_u32(&line[colons[1]+1..colons[2]]);
        if file_uid == uid {
            let name = &line[..colons[0]];
            let len = name.len().min(16);
            result[..len].copy_from_slice(&name[..len]);
            return result;
        }
    }
    // Fallback: numeric UID.
    let mut buf = [0u8; 10];
    let s = format_u32(uid, &mut buf);
    let len = s.len().min(16);
    result[..len].copy_from_slice(&s.as_bytes()[..len]);
    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_proc_path(path: &mut [u8; 48], pid: &[u8], suffix: &[u8]) -> usize {
    let prefix = b"/proc/";
    let mut i = 0;
    for &b in prefix { path[i] = b; i += 1; }
    for &b in pid { path[i] = b; i += 1; }
    for &b in suffix { path[i] = b; i += 1; }
    path[i] = 0;
    i
}

fn read_file_to_buf(file: SpecialFile, buf: &mut [u8]) -> usize {
    let path = Environment::get_special_file(file);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return 0; }
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n > 0 { n as usize } else { 0 }
}

fn parse_u32(bytes: &[u8]) -> u32 {
    let mut val: u32 = 0;
    for &b in bytes {
        if b >= b'0' && b <= b'9' { val = val * 10 + (b - b'0') as u32; }
    }
    val
}

fn format_u32(mut val: u32, buf: &mut [u8; 10]) -> &str {
    if val == 0 {
        buf[9] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[9..]) };
    }
    let mut i = 9;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        if i == 0 { break; }
        i -= 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[i + 1..]) }
}

fn pad_left(s: &str, width: usize) {
    let len = s.len();
    if len < width { for _ in 0..(width - len) { out(b" "); } }
    raw::raw_write(1, s.as_ptr(), s.len());
}

fn pad_right(s: &[u8; 16], width: usize) {
    // Find actual content length (trim trailing spaces).
    let len = s.iter().rposition(|&b| b != b' ').map(|p| p + 1).unwrap_or(0);
    raw::raw_write(1, s.as_ptr(), len);
    if len < width { for _ in 0..(width - len) { out(b" "); } }
}

fn out(b: &[u8]) {
    raw::raw_write(1, b.as_ptr(), b.len());
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
