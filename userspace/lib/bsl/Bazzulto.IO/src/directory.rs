//! Directory listing — enumerates VFS entries via getdents64.
//!
//! `read_dir(path)` opens the directory at `path`, calls getdents64 in a loop,
//! and returns the names of all entries (excluding "." and "..").
//!
//! The returned names are bare (not full paths).  Callers that need full paths
//! should prefix them with the directory path.

use bazzulto_system::raw;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// getdents64 record layout (matches kernel sys_getdents64):
//   offset  0: d_ino    (u64, 8 bytes)
//   offset  8: d_off    (u64, 8 bytes)
//   offset 16: d_reclen (u16, 2 bytes)
//   offset 18: d_type   (u8,  1 byte)
//   offset 19: d_name   (null-terminated string)
// ---------------------------------------------------------------------------

const DIRENT_HEADER_SIZE: usize = 19;

/// Read all entry names in the directory at `path`.
///
/// Returns bare entry names (no path prefix). "." and ".." are excluded.
/// Returns an empty Vec if the directory cannot be opened or is empty.
pub fn read_dir(path: &str) -> Vec<String> {
    let fd = raw::raw_open(path.as_ptr(), path.len());
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as i32;

    let mut entries = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = raw::raw_getdents64(fd, buf.as_mut_ptr(), buf.len());
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let mut offset = 0usize;
        while offset < n {
            if offset + DIRENT_HEADER_SIZE > n {
                break;
            }
            let reclen = u16::from_ne_bytes([buf[offset + 16], buf[offset + 17]]) as usize;
            if reclen == 0 || offset + reclen > n {
                break;
            }
            // Name starts at offset + 19, null-terminated.
            let name_start = offset + DIRENT_HEADER_SIZE;
            let name_end = buf[name_start..offset + reclen]
                .iter()
                .position(|&b| b == 0)
                .map(|pos| name_start + pos)
                .unwrap_or(offset + reclen);
            let name = core::str::from_utf8(&buf[name_start..name_end]).unwrap_or("");
            if !name.is_empty() && name != "." && name != ".." {
                entries.push(name.to_string());
            }
            offset += reclen;
        }
    }

    raw::raw_close(fd);
    entries
}

/// Directory listing facade.
pub struct Directory;

impl Directory {
    /// List all entries in `dir_path` whose name ends with `suffix`.
    ///
    /// Returns full paths: `"{dir_path}{name}"`.
    /// `dir_path` should end with `/`.
    pub fn list_with_suffix_in(dir_path: &str, suffix: &str) -> Vec<String> {
        read_dir(dir_path)
            .into_iter()
            .filter(|name| name.ends_with(suffix))
            .map(|name| {
                let mut full = String::from(dir_path);
                full.push_str(&name);
                full
            })
            .collect()
    }

    /// Collect all entries in `dir_path` whose name starts with `prefix`.
    pub fn list_with_prefix_in(dir_path: &str, prefix: &str) -> Vec<String> {
        read_dir(dir_path)
            .into_iter()
            .filter(|name| name.starts_with(prefix))
            .map(|name| {
                let mut full = String::from(dir_path);
                full.push_str(&name);
                full
            })
            .collect()
    }

    /// List all entry names in `dir_path` (bare names, no path prefix).
    pub fn read_dir(dir_path: &str) -> Vec<String> {
        read_dir(dir_path)
    }

    // ---------------------------------------------------------------------------
    // Legacy ramfs LIST-based methods — kept for compatibility.
    // These only see entries registered in ramfs, not VFS/FAT32 files.
    // ---------------------------------------------------------------------------

    /// Collect all ramfs entries whose name ends with `suffix`.
    #[deprecated(note = "use list_with_suffix_in(dir_path, suffix) for VFS directories")]
    pub fn list_with_suffix(suffix: &str) -> Vec<String> {
        let mut buffer = [0u8; 4096];
        let result = raw::raw_list(buffer.as_mut_ptr(), buffer.len());
        if result <= 0 {
            return Vec::new();
        }
        let bytes = &buffer[..result as usize];
        let text = core::str::from_utf8(bytes).unwrap_or("");
        text.split('\n')
            .filter(|s| !s.is_empty() && s.ends_with(suffix))
            .map(|s| s.to_string())
            .collect()
    }

    /// Collect all ramfs entries whose name starts with `prefix`.
    #[deprecated(note = "use list_with_prefix_in(dir_path, prefix) for VFS directories")]
    pub fn list_with_prefix(prefix: &str) -> Vec<String> {
        let mut buffer = [0u8; 4096];
        let result = raw::raw_list(buffer.as_mut_ptr(), buffer.len());
        if result <= 0 {
            return Vec::new();
        }
        let bytes = &buffer[..result as usize];
        let text = core::str::from_utf8(bytes).unwrap_or("");
        text.split('\n')
            .filter(|s| !s.is_empty() && s.starts_with(prefix))
            .map(|s| s.to_string())
            .collect()
    }

    /// Collect all registered ramfs entries.
    #[deprecated(note = "use read_dir(dir_path) for VFS directories")]
    pub fn list_all() -> Vec<String> {
        let mut buffer = [0u8; 4096];
        let result = raw::raw_list(buffer.as_mut_ptr(), buffer.len());
        if result <= 0 {
            return Vec::new();
        }
        let bytes = &buffer[..result as usize];
        let text = core::str::from_utf8(bytes).unwrap_or("");
        text.split('\n')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}
