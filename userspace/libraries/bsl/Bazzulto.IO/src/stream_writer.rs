//! Buffered text-oriented writer wrapping any `Write` implementation.
//!
//! Corresponds to `System.IO.StreamWriter` in the .NET BCL.
//! Works with any type that implements `crate::io_traits::Write`, including
//! `File` and `MemoryStream`.  No heap allocation is required for numeric
//! formatting; digits are produced directly into a stack buffer.

use crate::io_traits::Write;

// ---------------------------------------------------------------------------
// StreamWriter
// ---------------------------------------------------------------------------

/// A text-oriented writer that wraps any `W: Write`.
///
/// Provides string and line writing helpers, and decimal `u64` formatting
/// without depending on `format!` or the Rust standard library.
pub struct StreamWriter<W: Write> {
    inner: W,
}

impl<W: Write> StreamWriter<W> {
    /// Wrap `writer` in a `StreamWriter`.
    pub fn new(writer: W) -> Self {
        StreamWriter { inner: writer }
    }

    /// Consume the `StreamWriter` and return the inner writer.
    pub fn into_inner(self) -> W {
        self.inner
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Write the UTF-8 bytes of `string` to the underlying writer.
    pub fn write_str(&mut self, string: &str) -> Result<(), i32> {
        self.inner.write_all(string.as_bytes())
    }

    /// Write `string` followed by a `'\n'` byte.
    pub fn write_line(&mut self, string: &str) -> Result<(), i32> {
        self.inner.write_all(string.as_bytes())?;
        self.inner.write_all(b"\n")
    }

    /// Write the decimal representation of `number` without using `format!`.
    ///
    /// Digits are computed into a 20-byte stack buffer (sufficient for
    /// `u64::MAX` = 18_446_744_073_709_551_615) and written in a single call.
    pub fn write_fmt_u64(&mut self, number: u64) -> Result<(), i32> {
        // Stack buffer — 20 bytes covers the largest possible u64.
        let mut digits = [0u8; 20];
        let mut write_index = digits.len();

        if number == 0 {
            return self.inner.write_all(b"0");
        }

        let mut remaining = number;
        while remaining > 0 {
            write_index -= 1;
            digits[write_index] = b'0' + (remaining % 10) as u8;
            remaining /= 10;
        }

        self.inner.write_all(&digits[write_index..])
    }

    /// Flush any buffered bytes to the underlying sink.
    ///
    /// Because `StreamWriter` does not introduce its own buffer, this simply
    /// delegates to the inner writer's `flush`.
    pub fn flush(&mut self) -> Result<(), i32> {
        self.inner.flush()
    }
}
