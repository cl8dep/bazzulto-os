//! Bazzulto.IO — typed I/O primitives for Bazzulto userspace.
//!
//! Provides `File`, `Directory`, `Stream` (stdin/stdout/stderr), `Path`
//! helpers, trait-based I/O abstractions, info types, binary codecs,
//! and a `MemoryStream`. All syscall access goes through
//! `bazzulto_system::raw`.

#![no_std]

extern crate alloc;

// ---------------------------------------------------------------------------
// Core modules (original)
// ---------------------------------------------------------------------------

pub mod directory;
pub mod file;
pub mod path;
pub mod stream;

// ---------------------------------------------------------------------------
// New modules
// ---------------------------------------------------------------------------

/// `Read`, `Write`, `Seek` traits and `SeekFrom` — mirrors `System.IO.Stream`.
pub mod io_traits;

/// In-memory byte stream — mirrors `System.IO.MemoryStream`.
pub mod memory_stream;

/// Line-oriented reader — mirrors `System.IO.StreamReader`.
pub mod stream_reader;

/// Text/byte writer — mirrors `System.IO.StreamWriter`.
pub mod stream_writer;

/// Little-endian binary reader — mirrors `System.IO.BinaryReader`.
pub mod binary_reader;

/// Little-endian binary writer — mirrors `System.IO.BinaryWriter`.
pub mod binary_writer;

/// Path-based file operations — mirrors `System.IO.FileInfo`.
pub mod file_info;

/// Path-based directory operations — mirrors `System.IO.DirectoryInfo`.
pub mod directory_info;

/// Mounted filesystem information — mirrors `System.IO.DriveInfo`.
pub mod drive_info;

/// Filesystem change notification stub — mirrors `System.IO.FileSystemWatcher`.
pub mod filesystem_watcher;

// ---------------------------------------------------------------------------
// Re-exports — original
// ---------------------------------------------------------------------------

pub use file::File;
pub use directory::Directory;
pub use path::Path;
pub use stream::{stdin, stdout, stderr, Stream};

// ---------------------------------------------------------------------------
// Re-exports — new
// ---------------------------------------------------------------------------

pub use io_traits::{Read, Write, Seek, SeekFrom};
pub use memory_stream::MemoryStream;
pub use stream_reader::StreamReader;
pub use stream_writer::StreamWriter;
pub use binary_reader::BinaryReader;
pub use binary_writer::BinaryWriter;
pub use file_info::FileInfo;
pub use directory_info::DirectoryInfo;
pub use drive_info::{DriveInfo, DriveType};
pub use filesystem_watcher::FileSystemWatcher;

// ---------------------------------------------------------------------------
// mount — mount a filesystem at a VFS path
// ---------------------------------------------------------------------------

/// Mount a filesystem.
///
/// `source`: Bazzulto Path Model device path (e.g. `"//dev:diskb:1/"`).
/// `target`: mountpoint path (e.g. `"//home:user/"` or `"/home/user"`).
/// `fstype`: filesystem type (`"fat32"`, `"bafs"`, `"tmpfs"`).
///
/// Returns `Ok(())` on success or `Err(errno)` on failure.
pub fn mount(source: &str, target: &str, fstype: &str) -> Result<(), i32> {
    let ret = bazzulto_system::raw::raw_mount(
        source.as_ptr(), source.len(),
        target.as_ptr(), target.len(),
        fstype.as_ptr(), fstype.len(),
    );
    if ret < 0 { Err((-ret) as i32) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// MountInfo — parsed entry from sys_getmounts
// ---------------------------------------------------------------------------

/// A single entry returned by `getmounts()`.
#[derive(Clone)]
pub struct MountInfo {
    /// Absolute VFS mountpoint path (e.g. `"/"`, `"/home/user"`).
    pub mountpoint: alloc::string::String,
    /// Source device path in Bazzulto Path Model (e.g. `"//dev:diska:1/"`).
    /// Empty for virtual filesystems (tmpfs, devfs, procfs).
    pub source:     alloc::string::String,
    /// Filesystem type string (e.g. `"fat32"`, `"tmpfs"`).
    pub fstype:     alloc::string::String,
    /// Total capacity in 512-byte blocks. 0 for virtual filesystems.
    pub total_blocks: u64,
    /// Free (available) 512-byte blocks. 0 for virtual filesystems.
    pub free_blocks:  u64,
}

/// Return the list of all mounted filesystems.
///
/// Calls `sys_getmounts` with a two-pass strategy: first query the required
/// buffer size, then allocate and fill.
///
/// Returns an empty `Vec` on failure (kernel error or parse failure).
pub fn getmounts() -> alloc::vec::Vec<MountInfo> {
    use alloc::vec::Vec;
    use alloc::string::String;
    use bazzulto_system::raw::raw_getmounts;

    // Pass 1: query required size.
    let size = raw_getmounts(core::ptr::null_mut(), 0);
    if size <= 0 {
        return Vec::new();
    }

    // Allocate buffer.
    let mut buf: Vec<u8> = Vec::with_capacity(size as usize);
    // Safety: Vec::with_capacity allocates but does not initialise; we set_len after fill.
    unsafe { buf.set_len(size as usize); }

    // Pass 2: fill buffer.
    let written = raw_getmounts(buf.as_mut_ptr(), buf.len());
    if written <= 0 || written as usize > buf.len() {
        return Vec::new();
    }

    // Parse the flat buffer.
    let mut entries = Vec::new();
    let data = &buf[..written as usize];
    let mut pos = 0usize;

    while pos < data.len() {
        // Read mountpoint.
        if pos >= data.len() { break; }
        let mp_len = data[pos] as usize;
        pos += 1;
        if pos + mp_len > data.len() { break; }
        let mountpoint = match core::str::from_utf8(&data[pos..pos + mp_len]) {
            Ok(s) => String::from(s),
            Err(_) => break,
        };
        pos += mp_len;

        // Read source.
        if pos >= data.len() { break; }
        let src_len = data[pos] as usize;
        pos += 1;
        if pos + src_len > data.len() { break; }
        let source = match core::str::from_utf8(&data[pos..pos + src_len]) {
            Ok(s) => String::from(s),
            Err(_) => break,
        };
        pos += src_len;

        // Read fstype.
        if pos >= data.len() { break; }
        let fs_len = data[pos] as usize;
        pos += 1;
        if pos + fs_len > data.len() { break; }
        let fstype = match core::str::from_utf8(&data[pos..pos + fs_len]) {
            Ok(s) => String::from(s),
            Err(_) => break,
        };
        pos += fs_len;

        // Read total_blocks (u64 LE).
        if pos + 8 > data.len() { break; }
        let total_blocks = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap_or([0u8; 8]));
        pos += 8;

        // Read free_blocks (u64 LE).
        if pos + 8 > data.len() { break; }
        let free_blocks = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap_or([0u8; 8]));
        pos += 8;

        entries.push(MountInfo { mountpoint, source, fstype, total_blocks, free_blocks });
    }

    entries
}

/// Convert a Bazzulto canonical path (`//home:user/`) to a POSIX path (`/home/user`).
///
/// Rules:
///   - Paths starting with `//` are canonical: strip one leading `/`, then replace
///     every `:` with `/`, and strip any trailing `/`.
///   - Paths already in POSIX form are returned unchanged.
///
/// The result is written into `out` and the used length is returned.
fn canonical_to_posix<'a>(path: &'a str, out: &'a mut [u8]) -> &'a str {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'/' && bytes[1] == b'/' {
        // Strip the first `/` so we start from the second `/`.
        let inner = &bytes[1..]; // now "/home:user/"
        let mut len = 0usize;
        for &b in inner {
            if len >= out.len() { break; }
            out[len] = if b == b':' { b'/' } else { b };
            len += 1;
        }
        // Strip trailing slash (unless the path is exactly "/").
        while len > 1 && out[len - 1] == b'/' {
            len -= 1;
        }
        core::str::from_utf8(&out[..len]).unwrap_or(path)
    } else {
        path
    }
}

/// Create a directory and all missing parent directories.
///
/// Accepts both Bazzulto canonical paths (`//home:user/`) and POSIX paths
/// (`/home/user`).  Canonical paths are converted to POSIX before any mkdir
/// call so that directory names never contain `:` characters.
///
/// Ignores errors from individual `mkdir` calls (directory may already exist).
pub fn create_dir_all(path: &str) {
    let mut posix_buf = [0u8; 512];
    let posix = canonical_to_posix(path, &mut posix_buf);

    let mut i = 1usize;
    let bytes = posix.as_bytes();
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'/' {
            // SAFETY: posix is valid UTF-8 up to index i.
            if let Ok(prefix) = core::str::from_utf8(&bytes[..i]) {
                let _ = bazzulto_system::raw::raw_mkdir(prefix.as_ptr(), prefix.len(), 0o755);
            }
        }
        i += 1;
    }
}
