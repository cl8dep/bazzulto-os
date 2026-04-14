//! Little-endian binary primitive writer wrapping any `Write` implementation.
//!
//! Corresponds to `System.IO.BinaryWriter` in the .NET BCL.
//! All multi-byte integers are written in little-endian byte order, matching
//! the AArch64 default data endianness used by Bazzulto.

use crate::io_traits::Write;

// ---------------------------------------------------------------------------
// BinaryWriter
// ---------------------------------------------------------------------------

/// A little-endian binary primitive writer that wraps any `W: Write`.
///
/// Each `write_*` method writes exactly the number of bytes for that type.
/// Returns the inner writer's error on failure.
pub struct BinaryWriter<W: Write> {
    inner: W,
}

impl<W: Write> BinaryWriter<W> {
    /// Wrap `writer` in a `BinaryWriter`.
    pub fn new(writer: W) -> Self {
        BinaryWriter { inner: writer }
    }

    /// Consume the `BinaryWriter` and return the inner writer.
    pub fn into_inner(self) -> W {
        self.inner
    }

    // -----------------------------------------------------------------------
    // Unsigned integers
    // -----------------------------------------------------------------------

    /// Write one byte as a `u8`.
    pub fn write_u8(&mut self, value: u8) -> Result<(), i32> {
        self.inner.write_all(&[value])
    }

    /// Write `value` as two little-endian bytes.
    pub fn write_u16_le(&mut self, value: u16) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    /// Write `value` as four little-endian bytes.
    pub fn write_u32_le(&mut self, value: u32) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    /// Write `value` as eight little-endian bytes.
    pub fn write_u64_le(&mut self, value: u64) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    // -----------------------------------------------------------------------
    // Signed integers
    // -----------------------------------------------------------------------

    /// Write `value` as one byte (bit pattern preserved).
    pub fn write_i8(&mut self, value: i8) -> Result<(), i32> {
        self.inner.write_all(&[value as u8])
    }

    /// Write `value` as two little-endian bytes.
    pub fn write_i16_le(&mut self, value: i16) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    /// Write `value` as four little-endian bytes.
    pub fn write_i32_le(&mut self, value: i32) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    /// Write `value` as eight little-endian bytes.
    pub fn write_i64_le(&mut self, value: i64) -> Result<(), i32> {
        self.inner.write_all(&value.to_le_bytes())
    }

    // -----------------------------------------------------------------------
    // Boolean
    // -----------------------------------------------------------------------

    /// Write `false` as `0x00` and `true` as `0x01`.
    pub fn write_bool(&mut self, value: bool) -> Result<(), i32> {
        self.inner.write_all(&[if value { 1u8 } else { 0u8 }])
    }

    // -----------------------------------------------------------------------
    // Byte slice
    // -----------------------------------------------------------------------

    /// Write all bytes in `buf` to the underlying writer.
    pub fn write_bytes(&mut self, buf: &[u8]) -> Result<(), i32> {
        self.inner.write_all(buf)
    }

    // -----------------------------------------------------------------------
    // Flush
    // -----------------------------------------------------------------------

    /// Flush the underlying writer.
    pub fn flush(&mut self) -> Result<(), i32> {
        self.inner.flush()
    }
}
