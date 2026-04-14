//! I/O traits — `Read`, `Write`, `Seek` and `SeekFrom` for no_std environments.
//!
//! Corresponds to `System.IO.Stream` (abstract) and the core stream contracts
//! from the .NET BCL: `IDisposable`-owned read/write/seek split into three
//! orthogonal traits that any type may implement independently.
//!
//! All error values are negative-errno `i32` codes, matching the convention
//! used throughout Bazzulto.IO.

extern crate alloc;

use alloc::vec::Vec;
use crate::file::File;
use crate::file::{SEEK_SET, SEEK_CUR, SEEK_END};

// ---------------------------------------------------------------------------
// SeekFrom
// ---------------------------------------------------------------------------

/// Describes how a seek operation is positioned.
///
/// Mirrors `System.IO.SeekOrigin` but carries the offset inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekFrom {
    /// Seek to an absolute byte offset from the start of the stream.
    Start(u64),
    /// Seek relative to the end of the stream (offset may be negative).
    End(i64),
    /// Seek relative to the current cursor position.
    Current(i64),
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Trait for types that can be read byte-by-byte or in chunks.
///
/// Mirrors `System.IO.Stream.Read` / `System.IO.TextReader.Read`.
pub trait Read {
    /// Pull up to `buf.len()` bytes from the source into `buf`.
    ///
    /// Returns the number of bytes actually read.  A return value of `Ok(0)`
    /// signals end-of-stream.  Errors are negative errno codes.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, i32>;

    /// Read bytes until end-of-stream, appending them into `buf`.
    ///
    /// Returns the total number of bytes appended.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize, i32> {
        let mut chunk = [0u8; 512];
        let mut total = 0usize;
        loop {
            let bytes_read = self.read(&mut chunk)?;
            if bytes_read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..bytes_read]);
            total += bytes_read;
        }
        Ok(total)
    }

    /// Read exactly `buf.len()` bytes.
    ///
    /// Returns `Err(-5)` (EIO) if the stream ends before the buffer is filled.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), i32> {
        let mut filled = 0usize;
        while filled < buf.len() {
            let bytes_read = self.read(&mut buf[filled..])?;
            if bytes_read == 0 {
                return Err(-5); // EIO — unexpected EOF
            }
            filled += bytes_read;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

/// Trait for types that can accept byte data.
///
/// Mirrors `System.IO.Stream.Write` / `System.IO.TextWriter.Write`.
pub trait Write {
    /// Write bytes from `buf` to the sink.
    ///
    /// Returns the number of bytes actually written.  Short writes are valid;
    /// callers that require all bytes to be written should use `write_all`.
    fn write(&mut self, buf: &[u8]) -> Result<usize, i32>;

    /// Write all bytes in `buf`, retrying on short writes.
    fn write_all(&mut self, buf: &[u8]) -> Result<(), i32> {
        let mut remaining = buf;
        while !remaining.is_empty() {
            let written = self.write(remaining)?;
            if written == 0 {
                return Err(-5); // EIO — no progress
            }
            remaining = &remaining[written..];
        }
        Ok(())
    }

    /// Flush any buffered output to the underlying sink.
    ///
    /// The default implementation is a no-op because `File` writes are
    /// unbuffered (every write goes directly to the kernel).
    fn flush(&mut self) -> Result<(), i32> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Seek
// ---------------------------------------------------------------------------

/// Trait for types that support random-access cursor positioning.
///
/// Mirrors `System.IO.Stream.Seek`.
pub trait Seek {
    /// Move the cursor as described by `position`.
    ///
    /// Returns the new absolute byte offset from the start of the stream.
    fn seek(&mut self, position: SeekFrom) -> Result<u64, i32>;
}

// ---------------------------------------------------------------------------
// Blanket impls for File
// ---------------------------------------------------------------------------

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, i32> {
        // Delegate to File's inherent method (takes &self; valid because File
        // holds an fd and does not carry mutable cursor state of its own).
        File::read(self, buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize, i32> {
        File::write(self, buf)
    }
}

impl Seek for File {
    fn seek(&mut self, position: SeekFrom) -> Result<u64, i32> {
        let (offset, whence) = match position {
            SeekFrom::Start(n)   => (n as i64,  SEEK_SET),
            SeekFrom::End(n)     => (n,          SEEK_END),
            SeekFrom::Current(n) => (n,          SEEK_CUR),
        };
        let new_offset = File::seek(self, offset, whence)?;
        Ok(new_offset as u64)
    }
}
