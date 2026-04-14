//! Little-endian binary primitive reader wrapping any `Read` implementation.
//!
//! Corresponds to `System.IO.BinaryReader` in the .NET BCL.
//! All multi-byte integers are read in little-endian byte order, matching the
//! AArch64 default data endianness used by Bazzulto.

extern crate alloc;

use alloc::vec::Vec;
use crate::io_traits::Read;

// ---------------------------------------------------------------------------
// BinaryReader
// ---------------------------------------------------------------------------

/// A little-endian binary primitive reader that wraps any `R: Read`.
///
/// Each `read_*` method reads exactly the number of bytes for that type.
/// Returns `Err(-5)` (EIO) if the stream ends before the required bytes
/// are available.
pub struct BinaryReader<R: Read> {
    inner: R,
}

impl<R: Read> BinaryReader<R> {
    /// Wrap `reader` in a `BinaryReader`.
    pub fn new(reader: R) -> Self {
        BinaryReader { inner: reader }
    }

    /// Consume the `BinaryReader` and return the inner reader.
    pub fn into_inner(self) -> R {
        self.inner
    }

    // -----------------------------------------------------------------------
    // Internal helper
    // -----------------------------------------------------------------------

    /// Read exactly `N` bytes into a fixed-size array.
    fn read_exact_array<const N: usize>(&mut self) -> Result<[u8; N], i32> {
        let mut buffer = [0u8; N];
        self.inner.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    // -----------------------------------------------------------------------
    // Unsigned integers
    // -----------------------------------------------------------------------

    /// Read one byte as a `u8`.
    pub fn read_u8(&mut self) -> Result<u8, i32> {
        let bytes = self.read_exact_array::<1>()?;
        Ok(bytes[0])
    }

    /// Read two bytes as a little-endian `u16`.
    pub fn read_u16_le(&mut self) -> Result<u16, i32> {
        let bytes = self.read_exact_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read four bytes as a little-endian `u32`.
    pub fn read_u32_le(&mut self) -> Result<u32, i32> {
        let bytes = self.read_exact_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Read eight bytes as a little-endian `u64`.
    pub fn read_u64_le(&mut self) -> Result<u64, i32> {
        let bytes = self.read_exact_array::<8>()?;
        Ok(u64::from_le_bytes(bytes))
    }

    // -----------------------------------------------------------------------
    // Signed integers
    // -----------------------------------------------------------------------

    /// Read one byte as an `i8`.
    pub fn read_i8(&mut self) -> Result<i8, i32> {
        let bytes = self.read_exact_array::<1>()?;
        Ok(bytes[0] as i8)
    }

    /// Read two bytes as a little-endian `i16`.
    pub fn read_i16_le(&mut self) -> Result<i16, i32> {
        let bytes = self.read_exact_array::<2>()?;
        Ok(i16::from_le_bytes(bytes))
    }

    /// Read four bytes as a little-endian `i32`.
    pub fn read_i32_le(&mut self) -> Result<i32, i32> {
        let bytes = self.read_exact_array::<4>()?;
        Ok(i32::from_le_bytes(bytes))
    }

    /// Read eight bytes as a little-endian `i64`.
    pub fn read_i64_le(&mut self) -> Result<i64, i32> {
        let bytes = self.read_exact_array::<8>()?;
        Ok(i64::from_le_bytes(bytes))
    }

    // -----------------------------------------------------------------------
    // Boolean
    // -----------------------------------------------------------------------

    /// Read one byte; `0` → `false`, any non-zero value → `true`.
    pub fn read_bool(&mut self) -> Result<bool, i32> {
        Ok(self.read_u8()? != 0)
    }

    // -----------------------------------------------------------------------
    // Byte slice
    // -----------------------------------------------------------------------

    /// Read exactly `count` bytes into a new `Vec<u8>`.
    ///
    /// Returns `Err(-5)` (EIO) if the stream ends before `count` bytes are
    /// available.
    pub fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, i32> {
        let mut buffer = Vec::with_capacity(count);
        // Safety: we initialise all `count` bytes before any read.
        buffer.resize(count, 0u8);
        self.inner.read_exact(&mut buffer)?;
        Ok(buffer)
    }
}
