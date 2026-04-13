//! Typed file handle — open, read, write, seek, close, creat, unlink, fstat.

use bazzulto_system::raw;
use alloc::vec::Vec;
use alloc::string::String;

// ---------------------------------------------------------------------------
// Seek whence values (match kernel)
// ---------------------------------------------------------------------------

pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

// ---------------------------------------------------------------------------
// File
// ---------------------------------------------------------------------------

/// An open file descriptor. Closes automatically on drop.
pub struct File {
    fd: i32,
}

impl File {
    /// Open an existing file at `path`. Returns `Ok(File)` or `Err(errno)`.
    pub fn open(path: &str) -> Result<File, i32> {
        let result = raw::raw_open(path.as_ptr(), path.len());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(File { fd: result as i32 })
        }
    }

    /// Create (or truncate) a file at `path`. Returns `Ok(File)` or `Err(errno)`.
    pub fn create(path: &str) -> Result<File, i32> {
        let result = raw::raw_creat(path.as_ptr(), path.len());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(File { fd: result as i32 })
        }
    }

    /// Wrap a raw file descriptor (e.g. 0, 1, 2 for stdin/stdout/stderr).
    /// The caller must ensure the fd is valid and that `File` may close it.
    pub unsafe fn from_raw_fd(fd: i32) -> File {
        File { fd }
    }

    /// Raw fd — for passing to syscalls that expect `i32`.
    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }

    /// Read up to `buf.len()` bytes. Returns bytes read or `Err(errno)`.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, i32> {
        let result = raw::raw_read(self.fd, buf.as_mut_ptr(), buf.len());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }

    /// Write `buf`. Returns bytes written or `Err(errno)`.
    pub fn write(&self, buf: &[u8]) -> Result<usize, i32> {
        let result = raw::raw_write(self.fd, buf.as_ptr(), buf.len());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }

    /// Seek to `offset` relative to `whence` (SEEK_SET / SEEK_CUR / SEEK_END).
    /// Returns new offset or `Err(errno)`.
    pub fn seek(&self, offset: i64, whence: i32) -> Result<i64, i32> {
        let result = raw::raw_seek(self.fd, offset, whence);
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result)
        }
    }

    /// Read the entire file into a `Vec<u8>`.
    pub fn read_to_end(&self) -> Result<Vec<u8>, i32> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 512]; // keep stack pressure low — debug frames are large
        loop {
            let bytes_read = self.read(&mut chunk)?;
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..bytes_read]);
        }
        Ok(buffer)
    }

    /// Read the entire file as a UTF-8 string. Returns `Err(-1)` if not valid UTF-8.
    pub fn read_to_string(&self) -> Result<String, i32> {
        let bytes = self.read_to_end()?;
        String::from_utf8(bytes).map_err(|_| -1i32)
    }

    /// Write all bytes in `buf`, retrying short writes.
    pub fn write_all(&self, buf: &[u8]) -> Result<(), i32> {
        let mut remaining = buf;
        while !remaining.is_empty() {
            let written = self.write(remaining)?;
            remaining = &remaining[written..];
        }
        Ok(())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        raw::raw_close(self.fd);
    }
}

// ---------------------------------------------------------------------------
// Unlink (delete)
// ---------------------------------------------------------------------------

/// Delete a file at `path`. Returns `Ok(())` or `Err(errno)`.
pub fn unlink(path: &str) -> Result<(), i32> {
    let result = raw::raw_unlink(path.as_ptr(), path.len());
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(())
    }
}
