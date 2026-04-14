//! Buffered line-oriented reader wrapping any `Read` implementation.
//!
//! Corresponds to `System.IO.StreamReader` in the .NET BCL.
//! Works with any type that implements `crate::io_traits::Read`, including
//! `File` and `MemoryStream`.  No syscalls are issued by this type; I/O is
//! delegated entirely to the inner reader.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::io_traits::Read;

// ---------------------------------------------------------------------------
// StreamReader
// ---------------------------------------------------------------------------

/// A line-oriented reader that wraps any `R: Read`.
///
/// Maintains a small internal byte buffer to allow single-byte lookahead for
/// newline detection without issuing one syscall per character.
pub struct StreamReader<R: Read> {
    inner:  R,
    buffer: Vec<u8>,   // unconsumed bytes from the last kernel read
    offset: usize,     // first unconsumed byte in `buffer`
    done:   bool,      // true once the inner reader returned EOF
}

impl<R: Read> StreamReader<R> {
    /// Wrap `reader` in a `StreamReader`.
    pub fn new(reader: R) -> Self {
        StreamReader {
            inner:  reader,
            buffer: Vec::new(),
            offset: 0,
            done:   false,
        }
    }

    /// Consume the `StreamReader` and return the inner reader.
    pub fn into_inner(self) -> R {
        self.inner
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Refill `self.buffer` from the inner reader.
    /// Returns `false` when the inner reader is exhausted.
    fn refill(&mut self) -> bool {
        if self.done {
            return false;
        }
        // Discard already-consumed bytes.
        if self.offset > 0 {
            self.buffer.drain(..self.offset);
            self.offset = 0;
        }
        let start = self.buffer.len();
        // Append room for the next chunk.
        self.buffer.resize(start + 512, 0u8);
        match self.inner.read(&mut self.buffer[start..]) {
            Ok(0) | Err(_) => {
                self.buffer.truncate(start);
                self.done = true;
                false
            }
            Ok(n) => {
                self.buffer.truncate(start + n);
                true
            }
        }
    }

    /// Read one byte from the internal buffer, refilling if needed.
    /// Returns `None` at EOF.
    fn next_byte(&mut self) -> Option<u8> {
        loop {
            if self.offset < self.buffer.len() {
                let byte = self.buffer[self.offset];
                self.offset += 1;
                return Some(byte);
            }
            // Buffer exhausted — try to fill it.
            self.buffer.clear();
            self.offset = 0;
            if !self.refill() {
                return None;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Read bytes until a `'\n'` byte or EOF, returning the line as a `String`.
    ///
    /// The trailing `'\n'` is stripped.  A `'\r'` immediately before `'\n'` is
    /// also stripped (CRLF tolerance).  Returns `None` at EOF with no bytes
    /// read.
    pub fn read_line(&mut self) -> Option<String> {
        let mut line: Vec<u8> = Vec::new();
        loop {
            match self.next_byte() {
                None => {
                    if line.is_empty() {
                        return None;
                    }
                    break;
                }
                Some(b'\n') => break,
                Some(byte) => line.push(byte),
            }
        }
        // Strip trailing '\r' for CRLF compatibility.
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Some(bytes_to_string_lossy(line))
    }

    /// Read all remaining lines into a `Vec<String>`.
    ///
    /// Each element is one line with the trailing newline stripped.
    pub fn lines(&mut self) -> Vec<String> {
        let mut result = Vec::new();
        while let Some(line) = self.read_line() {
            result.push(line);
        }
        result
    }

    /// Read all remaining bytes into a `String`.
    ///
    /// Invalid UTF-8 bytes are replaced with `'?'`.
    pub fn read_to_string(&mut self) -> String {
        let mut all_bytes: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match self.inner.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => all_bytes.extend_from_slice(&chunk[..n]),
            }
        }
        bytes_to_string_lossy(all_bytes)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a byte vector to a String, replacing invalid UTF-8 with `'?'`.
fn bytes_to_string_lossy(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(error) => {
            let raw = error.into_bytes();
            let mut output = String::with_capacity(raw.len());
            let mut position = 0usize;
            while position < raw.len() {
                match core::str::from_utf8(&raw[position..]) {
                    Ok(valid) => {
                        output.push_str(valid);
                        break;
                    }
                    Err(utf8_error) => {
                        let valid_up_to = utf8_error.valid_up_to();
                        // SAFETY: valid_up_to is guaranteed to be a valid UTF-8 boundary.
                        let valid_part = unsafe {
                            core::str::from_utf8_unchecked(&raw[position..position + valid_up_to])
                        };
                        output.push_str(valid_part);
                        output.push('?');
                        let error_len = utf8_error.error_len().unwrap_or(1);
                        position += valid_up_to + error_len;
                    }
                }
            }
            output
        }
    }
}
