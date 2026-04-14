//! File metadata and convenience read/write operations by path.
//!
//! Corresponds to `System.IO.FileInfo` in the .NET BCL.
//! Because the Bazzulto kernel does not yet expose `fstat`, metadata fields
//! such as file size, creation time, and permissions cannot be queried.
//! Those are documented as `TODO(fstat)` below.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::raw;
use crate::file::File;

// ---------------------------------------------------------------------------
// FileInfo
// ---------------------------------------------------------------------------

/// Encapsulates path-based operations on a single file.
///
/// A `FileInfo` instance does not hold an open file descriptor; it stores only
/// the path string.  Methods that perform I/O open, operate, and close in a
/// single call.
pub struct FileInfo {
    path: String,
}

impl FileInfo {
    /// Create a `FileInfo` referring to the file at `path`.
    ///
    /// No I/O is performed at construction time.
    pub fn new(path: &str) -> Self {
        FileInfo { path: String::from(path) }
    }

    /// Return the full path stored in this `FileInfo`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Test whether the file exists by attempting to open and immediately close it.
    ///
    /// Returns `true` if the kernel returns a valid file descriptor.
    pub fn exists(&self) -> bool {
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let result = raw::raw_open(buf.as_ptr(), 0 /* O_RDONLY */, 0);
        if result >= 0 {
            raw::raw_close(result as i32);
            true
        } else {
            false
        }
    }

    /// Return the file name component: the substring after the last `/`.
    ///
    /// If there is no `/`, the whole path is returned.
    pub fn name(&self) -> &str {
        match self.path.rfind('/') {
            Some(position) => &self.path[position + 1..],
            None           => &self.path,
        }
    }

    /// Return the extension: the substring after the last `.` in the file name.
    ///
    /// Returns `None` for dotfiles (`.config`) and names without a dot.
    pub fn extension(&self) -> Option<&str> {
        let name = self.name();
        let dot_position = name.rfind('.')?;
        if dot_position == 0 {
            None // dotfile — no extension
        } else {
            Some(&name[dot_position + 1..])
        }
    }

    /// Return the parent directory path, or `None` if the path has no `/`.
    pub fn parent_directory(&self) -> Option<String> {
        let trimmed = self.path.trim_end_matches('/');
        let last_slash = trimmed.rfind('/')?;
        if last_slash == 0 {
            Some(String::from("/"))
        } else {
            Some(String::from(&trimmed[..last_slash]))
        }
    }

    // -----------------------------------------------------------------------
    // File handle constructors
    // -----------------------------------------------------------------------

    /// Open the file for reading. Returns `Err(errno)` on failure.
    pub fn open_read(&self) -> Result<File, i32> {
        File::open(&self.path)
    }

    /// Create or truncate the file and open it for writing.
    ///
    /// Equivalent to `File::create`.
    pub fn open_write(&self) -> Result<File, i32> {
        File::create(&self.path)
    }

    /// Open the file for appending (create if it does not exist, no truncation).
    pub fn open_append(&self) -> Result<File, i32> {
        // O_WRONLY=1 | O_CREAT=0x40 | O_APPEND=0x400
        let flags: i32 = 1 | 0x40 | 0x400;
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let result = raw::raw_open(buf.as_ptr(), flags, 0o666);
        if result < 0 {
            Err(result as i32)
        } else {
            // SAFETY: result is a valid fd returned by the kernel.
            Ok(unsafe { File::from_raw_fd(result as i32) })
        }
    }

    // -----------------------------------------------------------------------
    // Delete
    // -----------------------------------------------------------------------

    /// Delete the file. Returns `Err(errno)` on failure.
    pub fn delete(&self) -> Result<(), i32> {
        let mut buf = [0u8; 512];
        let len = self.path.len().min(511);
        buf[..len].copy_from_slice(&self.path.as_bytes()[..len]);
        let result = raw::raw_unlink(buf.as_ptr());
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    // -----------------------------------------------------------------------
    // Bulk read / write helpers
    // -----------------------------------------------------------------------

    /// Read the entire file contents into a `Vec<u8>`.
    pub fn read_all_bytes(&self) -> Result<Vec<u8>, i32> {
        let file = self.open_read()?;
        file.read_to_end()
    }

    /// Read the entire file as a UTF-8 string.
    ///
    /// Returns `Err(-1)` if the file contents are not valid UTF-8.
    pub fn read_all_text(&self) -> Result<String, i32> {
        let file = self.open_read()?;
        file.read_to_string()
    }

    /// Create or truncate the file and write `data` to it.
    pub fn write_all_bytes(&self, data: &[u8]) -> Result<(), i32> {
        let file = self.open_write()?;
        file.write_all(data)
    }

    /// Create or truncate the file and write the UTF-8 bytes of `text` to it.
    pub fn write_all_text(&self, text: &str) -> Result<(), i32> {
        self.write_all_bytes(text.as_bytes())
    }

    // -----------------------------------------------------------------------
    // Metadata via fstat (fd-based)
    // -----------------------------------------------------------------------

    /// Return the file size in bytes, or `Err(errno)` on failure.
    pub fn length_in_bytes(&self) -> Result<u64, i32> {
        let file = self.open_read()?;
        let mut stat_buf = [0u8; 128];
        let result = raw::raw_fstat(file.as_raw_fd(), stat_buf.as_mut_ptr());
        if result < 0 {
            return Err(result as i32);
        }
        // st_size is at offset 40 in the Linux stat64 struct (AArch64).
        let size = u64::from_le_bytes(stat_buf[40..48].try_into().unwrap_or([0u8; 8]));
        Ok(size)
    }

    /// Return the Unix mode bits (file type + permissions), or `Err(errno)`.
    pub fn permissions(&self) -> Result<u32, i32> {
        let file = self.open_read()?;
        let mut stat_buf = [0u8; 128];
        let result = raw::raw_fstat(file.as_raw_fd(), stat_buf.as_mut_ptr());
        if result < 0 {
            return Err(result as i32);
        }
        // st_mode is at offset 16 (u32).
        let mode = u32::from_le_bytes(stat_buf[16..20].try_into().unwrap_or([0u8; 4]));
        Ok(mode)
    }
}
