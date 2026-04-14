//! Mounted filesystem information.
//!
//! Corresponds to `System.IO.DriveInfo` in the .NET BCL.
//! Data is obtained by calling `sys_getmounts` via
//! `bazzulto_system::raw::raw_getmounts`, using the same two-pass
//! buffer-size-then-fill strategy and binary format as `crate::getmounts`.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::raw::raw_getmounts;

// ---------------------------------------------------------------------------
// DriveType
// ---------------------------------------------------------------------------

/// Classifies a mounted filesystem, mirroring `System.IO.DriveType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveType {
    /// The type cannot be determined from the available information.
    Unknown,
    /// A physical block device (e.g. `fat32`, `bafs`, `ext2`).
    Fixed,
    /// A memory-backed filesystem (e.g. `tmpfs`).
    Ram,
    /// A network-backed filesystem (e.g. `nfs`, `cifs`).
    Network,
}

// ---------------------------------------------------------------------------
// DriveInfo
// ---------------------------------------------------------------------------

/// Describes a single mounted filesystem.
///
/// Block counts use 512-byte sectors, matching the kernel `sys_getmounts`
/// output format.  Multiply by 512 to obtain byte values.
#[derive(Clone)]
pub struct DriveInfo {
    /// Absolute VFS mountpoint (e.g. `"/"`, `"/data"`).
    mountpoint:   String,
    /// Source device in Bazzulto Path Model (e.g. `"//dev:diska:1/"`).
    /// Empty for virtual filesystems.
    source:       String,
    /// Filesystem type string (e.g. `"fat32"`, `"tmpfs"`).
    fstype:       String,
    /// Total capacity in 512-byte blocks.
    total_blocks: u64,
    /// Available free space in 512-byte blocks.
    free_blocks:  u64,
}

impl DriveInfo {
    // -----------------------------------------------------------------------
    // Factory
    // -----------------------------------------------------------------------

    /// Return a list of all currently mounted filesystems.
    ///
    /// Uses the same two-pass strategy as `crate::getmounts()`:
    /// first query the required buffer size, then allocate and fill.
    ///
    /// Returns an empty `Vec` on failure.
    pub fn get_drives() -> Vec<DriveInfo> {
        // Pass 1: query required buffer size.
        let required_size = raw_getmounts(core::ptr::null_mut(), 0);
        if required_size <= 0 {
            return Vec::new();
        }

        // Allocate buffer.
        let mut buf: Vec<u8> = Vec::with_capacity(required_size as usize);
        // Safety: raw_getmounts will fill up to required_size bytes.
        unsafe { buf.set_len(required_size as usize); }

        // Pass 2: fill buffer.
        let bytes_written = raw_getmounts(buf.as_mut_ptr(), buf.len());
        if bytes_written <= 0 || bytes_written as usize > buf.len() {
            return Vec::new();
        }

        parse_getmounts_buffer(&buf[..bytes_written as usize])
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Return the mountpoint path (e.g. `"/"` or `"/home/user"`).
    pub fn name(&self) -> &str {
        &self.mountpoint
    }

    /// Classify the drive based on its filesystem type string.
    pub fn drive_type(&self) -> DriveType {
        match self.fstype.as_str() {
            "tmpfs" | "devfs" | "procfs" | "ramfs" => DriveType::Ram,
            "nfs" | "cifs" | "smbfs"               => DriveType::Network,
            "fat32" | "bafs" | "ext2" | "ext4"     => DriveType::Fixed,
            ""                                      => DriveType::Unknown,
            _                                       => DriveType::Fixed,
        }
    }

    /// Total capacity in bytes (`total_blocks * 512`).
    pub fn total_size(&self) -> u64 {
        self.total_blocks.saturating_mul(512)
    }

    /// Available free space in bytes (`free_blocks * 512`).
    pub fn available_free_space(&self) -> u64 {
        self.free_blocks.saturating_mul(512)
    }

    /// Used space in bytes.
    pub fn used_space(&self) -> u64 {
        self.total_size().saturating_sub(self.available_free_space())
    }

    /// Return `true` if the filesystem reports a non-zero total block count.
    ///
    /// Virtual filesystems (tmpfs, devfs) always return `false`.
    pub fn is_ready(&self) -> bool {
        self.total_blocks > 0
    }

    /// Return the source device label (Bazzulto Path Model).
    ///
    /// Empty for virtual filesystems.
    pub fn volume_label(&self) -> &str {
        &self.source
    }

    /// Return the filesystem type string (e.g. `"fat32"`, `"tmpfs"`).
    pub fn drive_format(&self) -> &str {
        &self.fstype
    }
}

// ---------------------------------------------------------------------------
// Buffer parser — mirrors the logic in crate::getmounts()
// ---------------------------------------------------------------------------

/// Parse the flat binary buffer produced by `sys_getmounts`.
///
/// Format per entry:
/// ```text
///   [u8: mountpoint_len][mountpoint_len bytes UTF-8]
///   [u8: source_len    ][source_len bytes UTF-8    ]
///   [u8: fstype_len    ][fstype_len bytes UTF-8    ]
///   [u64 LE: total_blocks]
///   [u64 LE: free_blocks ]
/// ```
fn parse_getmounts_buffer(data: &[u8]) -> Vec<DriveInfo> {
    let mut entries = Vec::new();
    let mut position = 0usize;

    while position < data.len() {
        // Read mountpoint.
        if position >= data.len() { break; }
        let mountpoint_length = data[position] as usize;
        position += 1;
        if position + mountpoint_length > data.len() { break; }
        let mountpoint = match core::str::from_utf8(&data[position..position + mountpoint_length]) {
            Ok(s)  => String::from(s),
            Err(_) => break,
        };
        position += mountpoint_length;

        // Read source.
        if position >= data.len() { break; }
        let source_length = data[position] as usize;
        position += 1;
        if position + source_length > data.len() { break; }
        let source = match core::str::from_utf8(&data[position..position + source_length]) {
            Ok(s)  => String::from(s),
            Err(_) => break,
        };
        position += source_length;

        // Read fstype.
        if position >= data.len() { break; }
        let fstype_length = data[position] as usize;
        position += 1;
        if position + fstype_length > data.len() { break; }
        let fstype = match core::str::from_utf8(&data[position..position + fstype_length]) {
            Ok(s)  => String::from(s),
            Err(_) => break,
        };
        position += fstype_length;

        // Read total_blocks (u64 LE).
        if position + 8 > data.len() { break; }
        let total_blocks = u64::from_le_bytes(
            data[position..position + 8].try_into().unwrap_or([0u8; 8])
        );
        position += 8;

        // Read free_blocks (u64 LE).
        if position + 8 > data.len() { break; }
        let free_blocks = u64::from_le_bytes(
            data[position..position + 8].try_into().unwrap_or([0u8; 8])
        );
        position += 8;

        entries.push(DriveInfo { mountpoint, source, fstype, total_blocks, free_blocks });
    }

    entries
}
