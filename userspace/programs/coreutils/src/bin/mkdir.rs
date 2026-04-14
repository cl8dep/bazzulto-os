// POSIX.1-2024 — mkdir
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/mkdir.html
//
// Create directories.
//
// Options:
//   -m mode   Set permission bits of the created directory. The mode argument
//             follows the same syntax as the chmod utility mode operand.
//             Accepted formats: octal (e.g. 755) or symbolic (e.g. u=rwx,go=rx).
//             In symbolic mode '+'/'-' are relative to the assumed base a=rwx (0777).
//   -p        Create any missing intermediate pathname components. Existing
//             directories are silently ignored. Intermediate directories are
//             created with mode (S_IWUSR|S_IXUSR|~umask)&0777 then chmod'd.
//
// Exit codes (POSIX §mkdir EXIT STATUS):
//   0   All specified directories created successfully, or -p specified and all
//       directories either already existed or were created successfully.
//  >0   An error occurred.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stderr};

// ---------------------------------------------------------------------------
// Symbolic mode parsing  (chmod mode operand — POSIX §chmod DESCRIPTION)
// ---------------------------------------------------------------------------
//
// Grammar (simplified):
//   symbolic_mode  := clause (',' clause)*
//   clause         := who* op permission*
//   who            := 'u' | 'g' | 'o' | 'a'
//   op             := '+' | '-' | '='
//   permission     := 'r' | 'w' | 'x' | 's' | 't' | 'X' | 'u' | 'g' | 'o'
//
// '+' and '-' are relative to the assumed initial mode a=rwx (0777) when the
// input begins with a symbolic clause (per POSIX §mkdir -m rationale).

/// Parse a mode string into a raw octal permission mask (bits [8:0] of st_mode).
///
/// Accepts:
///   - Octal strings:   "755", "0644"
///   - Symbolic strings: "u=rwx,go=rx", "a+w", "o-x"
///
/// Returns `None` on parse failure.
fn parse_mode(mode_str: &str) -> Option<u32> {
    // Try octal first.
    if mode_str.bytes().all(|b| b >= b'0' && b <= b'7') {
        let mut value = 0u32;
        for byte in mode_str.bytes() {
            value = value.checked_mul(8)?.checked_add((byte - b'0') as u32)?;
        }
        return Some(value & 0o7777);
    }

    // Symbolic mode: start from assumed base a=rwx = 0777.
    let mut mode: u32 = 0o777;
    for clause in mode_str.split(',') {
        let clause = clause.trim();
        if clause.is_empty() { return None; }

        // Collect who-characters until we hit an op.
        let mut who_mask: u32 = 0;
        let mut pos = 0usize;
        for byte in clause.bytes() {
            match byte {
                b'u' => { who_mask |= 0o700; pos += 1; }
                b'g' => { who_mask |= 0o070; pos += 1; }
                b'o' => { who_mask |= 0o007; pos += 1; }
                b'a' => { who_mask |= 0o777; pos += 1; }
                _    => break,
            }
        }
        // Default: 'a' (all) when no who-chars.
        if who_mask == 0 { who_mask = 0o777; }

        if pos >= clause.len() { return None; }
        let op = clause.as_bytes()[pos];
        if op != b'+' && op != b'-' && op != b'=' { return None; }
        pos += 1;

        // Collect permissions.
        let mut perm_bits: u32 = 0;
        for byte in clause.as_bytes()[pos..].iter().copied() {
            let bits: u32 = match byte {
                b'r' => 0o444 & who_mask,
                b'w' => 0o222 & who_mask,
                b'x' => 0o111 & who_mask,
                b's' => 0o6000 & if who_mask & 0o700 != 0 { 0o4000 } else { 0 }
                             | 0o6000 & if who_mask & 0o070 != 0 { 0o2000 } else { 0 },
                b't' => 0o1000,
                b'X' => {
                    // 'X': execute only if directory or already executable.
                    if mode & 0o111 != 0 { 0o111 & who_mask } else { 0 }
                }
                b'u' => {
                    // Copy owner bits to the target who positions.
                    let owner_rwx = (mode >> 6) & 0o7;
                    let mut spread = 0u32;
                    if who_mask & 0o700 != 0 { spread |= owner_rwx << 6; }
                    if who_mask & 0o070 != 0 { spread |= owner_rwx << 3; }
                    if who_mask & 0o007 != 0 { spread |= owner_rwx; }
                    spread
                }
                b'g' => {
                    let group_rwx = (mode >> 3) & 0o7;
                    let mut spread = 0u32;
                    if who_mask & 0o700 != 0 { spread |= group_rwx << 6; }
                    if who_mask & 0o070 != 0 { spread |= group_rwx << 3; }
                    if who_mask & 0o007 != 0 { spread |= group_rwx; }
                    spread
                }
                b'o' => {
                    let other_rwx = mode & 0o7;
                    let mut spread = 0u32;
                    if who_mask & 0o700 != 0 { spread |= other_rwx << 6; }
                    if who_mask & 0o070 != 0 { spread |= other_rwx << 3; }
                    if who_mask & 0o007 != 0 { spread |= other_rwx; }
                    spread
                }
                _ => return None,
            };
            perm_bits |= bits;
        }

        match op {
            b'+' => mode |=  perm_bits,
            b'-' => mode &= !perm_bits,
            b'=' => {
                // Clear the who bits, then set perm_bits.
                mode &= !who_mask;
                mode |= perm_bits & who_mask;
            }
            _ => return None,
        }
    }

    Some(mode & 0o7777)
}

// ---------------------------------------------------------------------------
// errno helpers
// ---------------------------------------------------------------------------

const EEXIST: i64 = -17;
const ENOENT: i64 = -2;

fn write_errno_message(prefix: &str, path: &str, errno: i64) {
    write_stderr(prefix);
    write_stderr(path);
    write_stderr(": ");
    let msg = match errno {
        -1  => "Operation not permitted",
        -2  => "No such file or directory",
        -13 => "Permission denied",
        -17 => "File exists",
        -20 => "Not a directory",
        -22 => "Invalid argument",
        -28 => "No space left on device",
        -36 => "File name too long",
        _   => "Error",
    };
    write_stderr(msg);
    write_stderr("\n");
}

// ---------------------------------------------------------------------------
// Directory creation helpers
// ---------------------------------------------------------------------------

/// Check whether `path` already names an existing directory.
/// Opens the path, calls fd-based fstat, checks S_IFDIR in the mode field.
fn is_existing_directory(path: &str) -> bool {
    let mut path_buf = [0u8; 512];
    let path_len = path.len().min(511);
    path_buf[..path_len].copy_from_slice(&path.as_bytes()[..path_len]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 {
        return false;
    }
    // 128-byte Linux stat64 struct: mode at offset 16 (u32).
    let mut stat_buf = [0u8; 128];
    let ret = raw::raw_fstat(fd as i32, stat_buf.as_mut_ptr());
    raw::raw_close(fd as i32);
    if ret < 0 {
        return false;
    }
    let mode = u32::from_le_bytes(stat_buf[16..20].try_into().unwrap_or([0; 4]));
    // S_IFDIR = 0x4000 (POSIX / Linux stat.h).
    (mode & 0xF000) == 0x4000
}

/// Create all missing intermediate components of `path`, then create `path`
/// itself with `final_mode`. Intermediate directories get mode
/// `(S_IWUSR | S_IXUSR | ~umask) & 0777` per POSIX.
///
/// Returns true on success (or if `path` already exists as a directory).
fn make_with_parents(path: &str, final_mode: u32) -> bool {
    // Get the current umask so we can compute intermediate mode.
    // raw_umask(mask) sets the mask and returns the previous one.
    // Set to itself by reading, then restoring — standard trick.
    let current_umask = raw::raw_umask(0o022);
    raw::raw_umask(current_umask); // restore

    // Intermediate mode: (S_IWUSR | S_IXUSR | ~umask) & 0777
    // S_IWUSR = 0o200, S_IXUSR = 0o100; ~umask in 9-bit space = 0o777 & !umask
    let inter_mode = (0o300u32 | (0o777 & !current_umask)) & 0o777;

    let bytes = path.as_bytes();

    // Walk each path prefix, creating missing components.
    let mut index = if bytes.first() == Some(&b'/') { 1 } else { 0 };
    loop {
        // Advance to next separator or end.
        while index < bytes.len() && bytes[index] != b'/' {
            index += 1;
        }
        let is_final = index >= bytes.len();
        let component = match core::str::from_utf8(&bytes[..index]) {
            Ok(s) if !s.is_empty() => s,
            _ => {
                if is_final { break; }
                index += 1;
                continue;
            }
        };

        let mut component_buf = [0u8; 512];
        let component_len = component.len().min(511);
        component_buf[..component_len].copy_from_slice(&component.as_bytes()[..component_len]);
        let ret = if is_final {
            raw::raw_mkdir(component_buf.as_ptr(), final_mode)
        } else {
            raw::raw_mkdir(component_buf.as_ptr(), inter_mode)
        };

        if ret < 0 && ret != EEXIST {
            // For intermediate components, ENOENT means a non-directory
            // component is in the path — that's a real error.
            write_errno_message("mkdir: cannot create directory '", component, ret);
            return false;
        }

        if is_final {
            // If final component already existed as a dir, POSIX -p says
            // silently ignore it.
            if ret == EEXIST && is_existing_directory(component) {
                return true;
            }
            if ret == EEXIST {
                write_errno_message("mkdir: cannot create directory '", path, ret);
                return false;
            }
            return true;
        }

        index += 1; // skip the '/'
    }

    true
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut create_parents = false;
    let mut mode: u32 = 0o777; // default: a=rwx, kernel applies umask
    let mut mode_set = false;
    let mut operands: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    let mut end_of_options = false;
    let mut iter = arguments[1..].iter();

    while let Some(arg) = iter.next() {
        let s = arg.as_str();
        if end_of_options || !s.starts_with('-') || s == "-" {
            operands.push(s);
            continue;
        }
        if s == "--" {
            end_of_options = true;
            continue;
        }
        // Parse flags — may be combined like -pm 755.
        let flags = &s[1..];
        let mut chars = flags.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                'p' => create_parents = true,
                'm' => {
                    // -m mode: mode value follows either as rest of this
                    // token or as the next argument.
                    let rest: alloc::string::String = chars.collect();
                    let mode_str = if rest.is_empty() {
                        match iter.next() {
                            Some(next) => next.as_str(),
                            None => {
                                write_stderr("mkdir: option requires an argument -- 'm'\n");
                                write_stderr("usage: mkdir [-p] [-m mode] dir...\n");
                                raw::raw_exit(1);
                            }
                        }
                    } else {
                        // rest is borrowed from a local String — but we need
                        // to break out of the char loop now.
                        let owned = rest;
                        match parse_mode(&owned) {
                            Some(m) => { mode = m; mode_set = true; }
                            None => {
                                write_stderr("mkdir: invalid mode '");
                                write_stderr(&owned);
                                write_stderr("'\n");
                                raw::raw_exit(1);
                            }
                        }
                        break; // consumed the rest of the token
                    };
                    match parse_mode(mode_str) {
                        Some(m) => { mode = m; mode_set = true; }
                        None => {
                            write_stderr("mkdir: invalid mode '");
                            write_stderr(mode_str);
                            write_stderr("'\n");
                            raw::raw_exit(1);
                        }
                    }
                    break; // -m consumed the rest of the token or next arg
                }
                other => {
                    write_stderr("mkdir: invalid option -- '");
                    let mut tmp = [0u8; 4];
                    write_stderr(other.encode_utf8(&mut tmp));
                    write_stderr("'\n");
                    write_stderr("usage: mkdir [-p] [-m mode] dir...\n");
                    raw::raw_exit(1);
                }
            }
        }
    }

    if operands.is_empty() {
        write_stderr("mkdir: missing operand\n");
        write_stderr("usage: mkdir [-p] [-m mode] dir...\n");
        raw::raw_exit(1);
    }

    let mut any_error = false;

    for &dir in &operands {
        if create_parents {
            if !make_with_parents(dir, mode) {
                any_error = true;
            }
        } else {
            let mut dir_buf = [0u8; 512];
            let dir_len = dir.len().min(511);
            dir_buf[..dir_len].copy_from_slice(&dir.as_bytes()[..dir_len]);
            let ret = raw::raw_mkdir(dir_buf.as_ptr(), mode);
            if ret < 0 {
                write_errno_message("mkdir: cannot create directory '", dir, ret);
                any_error = true;
            }
        }
    }

    raw::raw_exit(if any_error { 1 } else { 0 });
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
