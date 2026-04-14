//! Directory metadata and enumeration by path.
//!
//! Corresponds to `System.IO.DirectoryInfo` in the .NET BCL.
//! Entry enumeration uses `raw_getdents64`, following the same dirent layout
//! as `crate::directory::read_dir`.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use bazzulto_system::raw;
use crate::file_info::FileInfo;

// ---------------------------------------------------------------------------
// getdents64 record layout — matches kernel sys_getdents64 and directory.rs:
//   offset  0: d_ino    (u64, 8 bytes)
//   offset  8: d_off    (u64, 8 bytes)
//   offset 16: d_reclen (u16, 2 bytes)
//   offset 18: d_type   (u8,  1 byte)
//   offset 19: d_name   (null-terminated string)
// ---------------------------------------------------------------------------

const DIRENT_HEADER_SIZE: usize = 19;

// d_type constants (matching Linux / Bazzulto kernel values).
const DT_DIR: u8 = 4;
const DT_REG: u8 = 8;

// ---------------------------------------------------------------------------
// DirectoryInfo
// ---------------------------------------------------------------------------

/// Encapsulates path-based operations on a single directory.
///
/// A `DirectoryInfo` instance does not hold an open file descriptor; it stores
/// only the path string.  Methods that perform I/O open, operate, and close in
/// a single call.
pub struct DirectoryInfo {
    path: String,
}

impl DirectoryInfo {
    /// Create a `DirectoryInfo` referring to the directory at `path`.
    ///
    /// No I/O is performed at construction time.
    pub fn new(path: &str) -> Self {
        DirectoryInfo { path: String::from(path) }
    }

    /// Return the full path stored in this `DirectoryInfo`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Test whether the directory exists by opening it and calling getdents64.
    ///
    /// Returns `true` if the kernel returns a valid file descriptor for the path.
    pub fn exists(&self) -> bool {
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let fd = raw::raw_open(buf.as_ptr(), 0x10000 /* O_DIRECTORY */, 0);
        if fd >= 0 {
            raw::raw_close(fd as i32);
            true
        } else {
            false
        }
    }

    /// Return the directory name component: the substring after the last `/`.
    ///
    /// If there is no `/`, the whole path is returned.
    pub fn name(&self) -> &str {
        let trimmed = self.path.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(position) => &trimmed[position + 1..],
            None           => trimmed,
        }
    }

    /// Return the parent `DirectoryInfo`, or `None` if already at root.
    pub fn parent(&self) -> Option<DirectoryInfo> {
        let trimmed = self.path.trim_end_matches('/');
        let last_slash = trimmed.rfind('/')?;
        let parent_path = if last_slash == 0 {
            String::from("/")
        } else {
            String::from(&trimmed[..last_slash])
        };
        Some(DirectoryInfo { path: parent_path })
    }

    // -----------------------------------------------------------------------
    // Create / delete
    // -----------------------------------------------------------------------

    /// Create this directory with mode `0o755`.
    ///
    /// Returns `Err(errno)` on failure (e.g. parent does not exist).
    pub fn create(&self) -> Result<(), i32> {
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let result = raw::raw_mkdir(buf.as_ptr(), 0o755);
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    /// Delete a single file inside this directory by `filename` (bare name, no path).
    pub fn delete_file(&self, filename: &str) -> Result<(), i32> {
        let full_path = self.child_path(filename);
        let mut buf = [0u8; 512];
        let len = full_path.len().min(511);
        buf[..len].copy_from_slice(&full_path.as_bytes()[..len]);
        let result = raw::raw_unlink(buf.as_ptr());
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    // -----------------------------------------------------------------------
    // Entry enumeration
    // -----------------------------------------------------------------------

    /// Return `FileInfo` instances for every regular-file entry in this directory.
    ///
    /// Entries with `d_type == DT_REG` are included; all others are skipped.
    /// Returns an empty `Vec` if the directory cannot be opened.
    pub fn get_files(&self) -> Vec<FileInfo> {
        self.enumerate_entries_by_type(DT_REG)
            .into_iter()
            .map(|name| FileInfo::new(&self.child_path(&name)))
            .collect()
    }

    /// Return `DirectoryInfo` instances for every subdirectory entry.
    ///
    /// Entries with `d_type == DT_DIR` (excluding `.` and `..`) are included.
    pub fn get_directories(&self) -> Vec<DirectoryInfo> {
        self.enumerate_entries_by_type(DT_DIR)
            .into_iter()
            .map(|name| DirectoryInfo::new(&self.child_path(&name)))
            .collect()
    }

    /// Return bare entry names for every entry (files and directories) in this directory.
    ///
    /// `.` and `..` are excluded.
    pub fn get_entries(&self) -> Vec<String> {
        read_dir_entries(&self.path)
    }

    /// Return `FileInfo` instances for every regular-file entry whose name ends
    /// with `extension` (e.g. `".txt"` or `".service"`).
    pub fn enumerate_files(&self, extension: &str) -> Vec<FileInfo> {
        read_dir_entries(&self.path)
            .into_iter()
            .filter(|name| name.ends_with(extension))
            .map(|name| FileInfo::new(&self.child_path(&name)))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Build a full child path: `self.path` + `/` + `name`.
    fn child_path(&self, name: &str) -> String {
        let mut full = self.path.clone();
        if !full.ends_with('/') {
            full.push('/');
        }
        full.push_str(name);
        full
    }

    /// Enumerate raw entry names filtered by `d_type`.
    fn enumerate_entries_by_type(&self, target_type: u8) -> Vec<String> {
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let fd = raw::raw_open(buf.as_ptr(), 0x10000 /* O_DIRECTORY */, 0);
        if fd < 0 {
            return Vec::new();
        }
        let fd = fd as i32;
        let result = getdents_filtered(fd, Some(target_type));
        raw::raw_close(fd);
        result
    }
}

// ---------------------------------------------------------------------------
// Module-private getdents helpers
// ---------------------------------------------------------------------------

/// Read all non-`.`/`..` entry names from an already-open directory fd.
///
/// If `type_filter` is `Some(t)`, only entries with `d_type == t` are
/// returned.  Pass `None` to collect all entry types.
fn getdents_filtered(fd: i32, type_filter: Option<u8>) -> Vec<String> {
    let mut entries = Vec::new();
    let mut buffer = [0u8; 4096];

    loop {
        let n = raw::raw_getdents64(fd, buffer.as_mut_ptr(), buffer.len());
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let mut offset = 0usize;
        while offset < n {
            if offset + DIRENT_HEADER_SIZE > n {
                break;
            }
            let record_length =
                u16::from_ne_bytes([buffer[offset + 16], buffer[offset + 17]]) as usize;
            if record_length == 0 || offset + record_length > n {
                break;
            }
            let entry_type = buffer[offset + 18];
            let name_start = offset + DIRENT_HEADER_SIZE;
            let name_end = buffer[name_start..offset + record_length]
                .iter()
                .position(|&b| b == 0)
                .map(|pos| name_start + pos)
                .unwrap_or(offset + record_length);
            let name = core::str::from_utf8(&buffer[name_start..name_end]).unwrap_or("");
            let include = !name.is_empty()
                && name != "."
                && name != ".."
                && type_filter.map_or(true, |t| entry_type == t);
            if include {
                entries.push(name.to_string());
            }
            offset += record_length;
        }
    }

    entries
}

/// Read all non-`.`/`..` entry names from the directory at `path`.
fn read_dir_entries(path: &str) -> Vec<String> {
    let mut buf = [0u8; 512];
    let len = path.len().min(511);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    let fd = raw::raw_open(buf.as_ptr(), 0x10000 /* O_DIRECTORY */, 0);
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as i32;
    let result = getdents_filtered(fd, None);
    raw::raw_close(fd);
    result
}
