//! Standard streams — thin wrappers around fd 0 (stdin), 1 (stdout), 2 (stderr).

use bazzulto_system::raw;

/// A standard I/O stream backed by a fixed file descriptor.
pub struct Stream {
    fd: i32,
}

impl Stream {
    /// Write bytes to the stream. Returns bytes written or `Err(errno)`.
    pub fn write(&self, buf: &[u8]) -> Result<usize, i32> {
        let result = raw::raw_write(self.fd, buf.as_ptr(), buf.len());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
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

    /// Write a string followed by `\n`.
    pub fn write_line(&self, line: &str) -> Result<(), i32> {
        self.write_all(line.as_bytes())?;
        self.write_all(b"\n")
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

    /// Return the underlying fd.
    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }
}

/// Standard input (fd 0).
pub fn stdin() -> Stream {
    Stream { fd: 0 }
}

/// Standard output (fd 1).
pub fn stdout() -> Stream {
    Stream { fd: 1 }
}

/// Standard error (fd 2).
pub fn stderr() -> Stream {
    Stream { fd: 2 }
}
