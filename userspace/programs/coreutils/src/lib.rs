//! Shared helpers for Bazzulto coreutils.
//!
//! All utilities are `#![no_std]` and use the Bazzulto System Library (BSL)
//! for I/O and syscalls.  This library provides common functionality so that
//! each utility binary remains small and focused.

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::raw;

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

pub fn write_stdout(message: &str) {
    raw::raw_write(1, message.as_ptr(), message.len());
}

pub fn write_stderr(message: &str) {
    raw::raw_write(2, message.as_ptr(), message.len());
}

pub fn write_bytes_stdout(bytes: &[u8]) {
    raw::raw_write(1, bytes.as_ptr(), bytes.len());
}

/// Print `message` to stderr and exit with code 1.
pub fn die(message: &str) -> ! {
    write_stderr(message);
    write_stderr("\n");
    raw::raw_exit(1)
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// Return the process arguments as an owned Vec<String>.
/// argv[0] is the program name (already skipped in most utils — callers
/// typically use `args()[1..]` for actual arguments).
pub fn args() -> Vec<String> {
    bazzulto_system::args()
        .map(|s| {
            let mut owned = String::new();
            owned.push_str(s);
            owned
        })
        .collect()
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

/// Read all bytes from stdin into a Vec<u8>.
pub fn read_stdin_to_end() -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = raw::raw_read(0, chunk.as_mut_ptr(), chunk.len());
        if n <= 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n as usize]);
    }
    buffer
}

/// Read all bytes from an open file descriptor into a Vec<u8>.
pub fn read_fd_to_end(fd: i32) -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = raw::raw_read(fd, chunk.as_mut_ptr(), chunk.len());
        if n <= 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n as usize]);
    }
    buffer
}

/// Open a file for reading. Returns the fd on success or an error string.
pub fn open_file(path: &str) -> Result<i32, &'static str> {
    let mut path_buf = [0u8; 512];
    let path_len = path.len().min(511);
    path_buf[..path_len].copy_from_slice(&path.as_bytes()[..path_len]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 {
        Err("cannot open file")
    } else {
        Ok(fd as i32)
    }
}

/// Read a named file or stdin (when `path` is `None` or `"-"`) into a Vec<u8>.
pub fn read_file_or_stdin(path: Option<&str>) -> Result<Vec<u8>, &'static str> {
    match path {
        None | Some("-") => Ok(read_stdin_to_end()),
        Some(p) => {
            let fd = open_file(p)?;
            let data = read_fd_to_end(fd);
            raw::raw_close(fd);
            Ok(data)
        }
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Split bytes into lines (by `\n`), returned as UTF-8 strings.
/// Lines that are not valid UTF-8 are skipped.
pub fn lines_from_bytes(bytes: &[u8]) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            let slice = &bytes[start..index];
            if let Ok(s) = core::str::from_utf8(slice) {
                lines.push(s);
            }
            start = index + 1;
        }
    }
    // Trailing content without a final newline.
    if start < bytes.len() {
        if let Ok(s) = core::str::from_utf8(&bytes[start..]) {
            lines.push(s);
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Number formatting (no_std — core only)
// ---------------------------------------------------------------------------

/// Format a u64 as decimal into a stack buffer and return a &str slice of it.
/// The buffer must be at least 20 bytes.
pub fn format_u64(value: u64, buffer: &mut [u8; 20]) -> &str {
    if value == 0 {
        buffer[19] = b'0';
        return core::str::from_utf8(&buffer[19..]).unwrap_or("0");
    }
    let mut cursor = 20usize;
    let mut remaining = value;
    while remaining > 0 {
        cursor -= 1;
        buffer[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    core::str::from_utf8(&buffer[cursor..]).unwrap_or("?")
}

/// Write a u64 to stdout as decimal.
pub fn write_u64(value: u64) {
    let mut buffer = [0u8; 20];
    let s = format_u64(value, &mut buffer);
    write_stdout(s);
}
