// fs/btrfs/crc32c.rs — CRC32C (Castagnoli) checksum for Btrfs metadata/data.
//
// Btrfs uses CRC32C (polynomial 0x1EDC6F41) for all on-disk checksums:
// superblock, B-tree node headers, and data extent checksums.
//
// This is a software table-based implementation.  AArch64 hardware CRC
// instructions (CRC32CX etc.) could be used if FEAT_CRC32 is enabled, but
// the software path is correct on all targets and fast enough for our
// single-device hobby-OS use case.
//
// Reference: Castagnoli et al., "Optimization of Cyclic Redundancy-Check
//            Codes with 24 and 32 Parity Bits", IEEE 1993.
//            Linux kernel: lib/crc32.c, crypto/crc32c_generic.c.

/// Precomputed CRC32C lookup table (256 entries, one per byte value).
///
/// Generated from the CRC32C polynomial 0x82F63B78 (bit-reversed form of
/// 0x1EDC6F41).
const CRC32C_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i: usize = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x82F63B78;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32C over `data`, starting from `seed`.
///
/// Btrfs convention:
/// - Seed is `0xFFFF_FFFF` for fresh checksums.
/// - The stored checksum is `!crc32c(seed, data)` (bitwise NOT of the raw
///   accumulator).
///
/// Example (superblock checksum):
/// ```ignore
/// let csum = crc32c(!0u32, &superblock_bytes[0x20..]);
/// let stored = !csum;  // write stored into superblock_bytes[0x00..0x04]
/// ```
pub fn crc32c(seed: u32, data: &[u8]) -> u32 {
    let mut crc = seed;
    for &byte in data {
        crc = CRC32C_TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// Compute the final Btrfs checksum value for a byte slice.
///
/// This is the form stored on disk: `!crc32c(!0, data)`.
pub fn btrfs_checksum(data: &[u8]) -> u32 {
    !crc32c(!0u32, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // CRC32C("123456789") = 0xE3069283
        let result = !crc32c(!0u32, b"123456789");
        assert_eq!(result, 0xE3069283);
    }

    #[test]
    fn empty_input() {
        let result = !crc32c(!0u32, b"");
        assert_eq!(result, 0x00000000);
    }
}
