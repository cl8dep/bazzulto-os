//! In-memory byte stream backed by a `Vec<u8>` with a cursor.
//!
//! Corresponds to `System.IO.MemoryStream` in the .NET BCL.
//! No syscalls are issued; all operations are pure in-process memory
//! manipulation.

extern crate alloc;

use alloc::vec::Vec;
use crate::io_traits::{Read, Write, Seek, SeekFrom};

// ---------------------------------------------------------------------------
// MemoryStream
// ---------------------------------------------------------------------------

/// A growable in-memory byte buffer with an explicit read/write cursor.
///
/// Wraps a `Vec<u8>` and tracks a `position` so that `Read`, `Write`, and
/// `Seek` all share a single logical stream view, matching the semantics of
/// `System.IO.MemoryStream`.
pub struct MemoryStream {
    data:     Vec<u8>,
    position: usize,
}

impl MemoryStream {
    /// Create an empty `MemoryStream` with the cursor at position 0.
    pub fn new() -> Self {
        MemoryStream {
            data:     Vec::new(),
            position: 0,
        }
    }

    /// Create a `MemoryStream` pre-populated with `data`, cursor at 0.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        MemoryStream { data, position: 0 }
    }

    /// Consume the stream and return the underlying `Vec<u8>`.
    pub fn into_inner(self) -> Vec<u8> {
        self.data
    }

    /// Return a slice of the entire buffer contents, regardless of cursor.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Return the current cursor position (bytes from the start).
    pub fn position(&self) -> u64 {
        self.position as u64
    }

    /// Return the total number of bytes currently in the buffer.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Return `true` if the buffer contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Reset the cursor to byte 0 without altering the buffer contents.
    pub fn reset(&mut self) {
        self.position = 0;
    }
}

impl Default for MemoryStream {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

impl Read for MemoryStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, i32> {
        if self.position >= self.data.len() {
            return Ok(0); // EOF
        }
        let available = self.data.len() - self.position;
        let to_copy = if buf.len() < available { buf.len() } else { available };
        buf[..to_copy].copy_from_slice(&self.data[self.position..self.position + to_copy]);
        self.position += to_copy;
        Ok(to_copy)
    }
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

impl Write for MemoryStream {
    /// Write `buf` at the current cursor position, extending the buffer if
    /// needed.  Overwrites existing bytes when writing within the current
    /// data extent, and appends beyond it.
    fn write(&mut self, buf: &[u8]) -> Result<usize, i32> {
        let end = self.position + buf.len();
        if end > self.data.len() {
            // Grow the buffer to accommodate the write.
            self.data.resize(end, 0u8);
        }
        self.data[self.position..end].copy_from_slice(buf);
        self.position = end;
        Ok(buf.len())
    }
}

// ---------------------------------------------------------------------------
// Seek
// ---------------------------------------------------------------------------

impl Seek for MemoryStream {
    fn seek(&mut self, position: SeekFrom) -> Result<u64, i32> {
        let length = self.data.len() as i64;
        let new_position: i64 = match position {
            SeekFrom::Start(n)   => n as i64,
            SeekFrom::End(n)     => length + n,
            SeekFrom::Current(n) => self.position as i64 + n,
        };
        if new_position < 0 {
            return Err(-22); // EINVAL — cannot seek before start
        }
        self.position = new_position as usize;
        Ok(self.position as u64)
    }
}
