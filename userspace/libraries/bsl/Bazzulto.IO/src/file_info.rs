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
        let result = raw::raw_open(self.path.as_ptr(), self.path.len());
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
    ///
    /// Uses `raw_creat_append` which passes `flags=1` to the kernel.
    pub fn open_append(&self) -> Result<File, i32> {
        let result = raw::raw_creat_append(self.path.as_ptr(), self.path.len());
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
        let result = raw::raw_unlink(self.path.as_ptr(), self.path.len());
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
    // TODO(fstat): the following methods require the kernel to expose a
    // `sys_fstat` (or equivalent `sys_stat`) syscall wired into the vDSO.
    //
    // Once SLOT_FSTAT is available via bazzulto_system::raw::raw_fstat():
    //   pub fn length_in_bytes(&self) -> Result<u64, i32>
    //   pub fn created_time(&self) -> Result<u64, i32>   // seconds since epoch
    //   pub fn modified_time(&self) -> Result<u64, i32>
    //   pub fn access_time(&self) -> Result<u64, i32>
    //   pub fn permissions(&self) -> Result<u32, i32>    // Unix mode bits
    // -----------------------------------------------------------------------
}
