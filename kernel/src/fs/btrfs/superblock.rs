// fs/btrfs/superblock.rs — Read and validate the Btrfs superblock.
//
// The primary superblock lives at byte offset 0x10000 (64 KiB) on the device.
// Mirror copies exist at 64 MiB and 256 GiB, but we only read the primary
// for now (single-device hobby OS — the mirrors are for recovery on real
// hardware).
//
// Reference: btrfs on-disk format §2 "Superblock".

extern crate alloc;

use alloc::vec;

use super::crc32c;
use super::ondisk::*;
use crate::hal::disk::BlockDevice;

/// Errors returned by superblock operations.
#[derive(Debug)]
pub enum SuperblockError {
    /// Could not read sectors from the block device.
    ReadError,
    /// Magic number mismatch — not a Btrfs filesystem.
    BadMagic,
    /// Checksum mismatch.
    BadChecksum,
    /// Unsupported checksum type (only CRC32C is supported).
    UnsupportedChecksumType,
    /// nodesize or sectorsize is zero or unreasonable.
    InvalidGeometry,
}

/// Read and validate the primary superblock from `disk` at `partition_start_lba`.
///
/// Returns the parsed `BtrfsSuperblock` on success.
pub fn read_superblock(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
) -> Result<BtrfsSuperblock, SuperblockError> {
    // The superblock is at byte offset 0x10000 within the partition.
    // With 512-byte sectors, that's sector 128.
    let sb_sector = partition_start_lba + (BTRFS_SUPER_INFO_OFFSET / 512);
    let sectors_needed = (BTRFS_SUPER_INFO_SIZE + 511) / 512; // 8 sectors

    let mut raw = vec![0u8; sectors_needed * 512];
    if !disk.read_sectors(sb_sector, sectors_needed as u32, &mut raw) {
        return Err(SuperblockError::ReadError);
    }

    // Validate magic.
    let magic = u64::from_le_bytes(raw[SB_OFF_MAGIC..SB_OFF_MAGIC + 8].try_into().unwrap());
    if magic != BTRFS_MAGIC {
        return Err(SuperblockError::BadMagic);
    }

    // Validate checksum (CRC32C of bytes 0x20 .. 0x1000).
    let csum_type = u16::from_le_bytes(raw[SB_OFF_CSUM_TYPE..SB_OFF_CSUM_TYPE + 2].try_into().unwrap());
    if csum_type != BTRFS_CSUM_TYPE_CRC32C {
        return Err(SuperblockError::UnsupportedChecksumType);
    }

    let computed = crc32c::btrfs_checksum(&raw[0x20..BTRFS_SUPER_INFO_SIZE]);
    let stored = u32::from_le_bytes(raw[0..4].try_into().unwrap());
    if computed != stored {
        return Err(SuperblockError::BadChecksum);
    }

    // Parse fields.
    let sb = parse_superblock(&raw)?;
    Ok(sb)
}

/// Parse a `BtrfsSuperblock` from a raw 4096-byte buffer.
fn parse_superblock(raw: &[u8]) -> Result<BtrfsSuperblock, SuperblockError> {
    let nodesize = u32::from_le_bytes(raw[SB_OFF_NODESIZE..SB_OFF_NODESIZE + 4].try_into().unwrap());
    let sectorsize = u32::from_le_bytes(raw[SB_OFF_SECTORSIZE..SB_OFF_SECTORSIZE + 4].try_into().unwrap());
    if nodesize == 0 || sectorsize == 0 || nodesize < sectorsize {
        return Err(SuperblockError::InvalidGeometry);
    }

    let sys_chunk_array_size = u32::from_le_bytes(
        raw[SB_OFF_SYS_CHUNK_ARRAY_SIZE..SB_OFF_SYS_CHUNK_ARRAY_SIZE + 4].try_into().unwrap()
    );

    let mut csum = [0u8; BTRFS_CSUM_SIZE];
    csum.copy_from_slice(&raw[SB_OFF_CSUM..SB_OFF_CSUM + BTRFS_CSUM_SIZE]);

    let mut fsid = [0u8; BTRFS_UUID_SIZE];
    fsid.copy_from_slice(&raw[SB_OFF_FSID..SB_OFF_FSID + BTRFS_UUID_SIZE]);

    let mut label = [0u8; BTRFS_LABEL_SIZE];
    let label_end = (SB_OFF_LABEL + BTRFS_LABEL_SIZE).min(raw.len());
    label[..label_end - SB_OFF_LABEL].copy_from_slice(&raw[SB_OFF_LABEL..label_end]);

    let mut metadata_uuid = [0u8; BTRFS_UUID_SIZE];
    metadata_uuid.copy_from_slice(&raw[SB_OFF_METADATA_UUID..SB_OFF_METADATA_UUID + BTRFS_UUID_SIZE]);

    let dev_item = BtrfsDevItem::from_bytes(&raw[SB_OFF_DEV_ITEM..SB_OFF_DEV_ITEM + BtrfsDevItem::SIZE]);

    let mut sys_chunk_array = [0u8; BTRFS_SYSTEM_CHUNK_ARRAY_SIZE];
    let sca_len = (sys_chunk_array_size as usize).min(BTRFS_SYSTEM_CHUNK_ARRAY_SIZE);
    sys_chunk_array[..sca_len].copy_from_slice(
        &raw[SB_OFF_SYS_CHUNK_ARRAY..SB_OFF_SYS_CHUNK_ARRAY + sca_len]
    );

    Ok(BtrfsSuperblock {
        csum,
        fsid,
        bytenr: u64::from_le_bytes(raw[SB_OFF_BYTENR..SB_OFF_BYTENR + 8].try_into().unwrap()),
        flags: u64::from_le_bytes(raw[SB_OFF_FLAGS..SB_OFF_FLAGS + 8].try_into().unwrap()),
        magic: u64::from_le_bytes(raw[SB_OFF_MAGIC..SB_OFF_MAGIC + 8].try_into().unwrap()),
        generation: u64::from_le_bytes(raw[SB_OFF_GENERATION..SB_OFF_GENERATION + 8].try_into().unwrap()),
        root: u64::from_le_bytes(raw[SB_OFF_ROOT..SB_OFF_ROOT + 8].try_into().unwrap()),
        chunk_root: u64::from_le_bytes(raw[SB_OFF_CHUNK_ROOT..SB_OFF_CHUNK_ROOT + 8].try_into().unwrap()),
        log_root: u64::from_le_bytes(raw[SB_OFF_LOG_ROOT..SB_OFF_LOG_ROOT + 8].try_into().unwrap()),
        total_bytes: u64::from_le_bytes(raw[SB_OFF_TOTAL_BYTES..SB_OFF_TOTAL_BYTES + 8].try_into().unwrap()),
        bytes_used: u64::from_le_bytes(raw[SB_OFF_BYTES_USED..SB_OFF_BYTES_USED + 8].try_into().unwrap()),
        root_dir_objectid: u64::from_le_bytes(raw[SB_OFF_ROOT_DIR_OBJECTID..SB_OFF_ROOT_DIR_OBJECTID + 8].try_into().unwrap()),
        num_devices: u64::from_le_bytes(raw[SB_OFF_NUM_DEVICES..SB_OFF_NUM_DEVICES + 8].try_into().unwrap()),
        sectorsize,
        nodesize,
        stripesize: u32::from_le_bytes(raw[SB_OFF_STRIPESIZE..SB_OFF_STRIPESIZE + 4].try_into().unwrap()),
        sys_chunk_array_size,
        chunk_root_generation: u64::from_le_bytes(raw[SB_OFF_CHUNK_ROOT_GENERATION..SB_OFF_CHUNK_ROOT_GENERATION + 8].try_into().unwrap()),
        compat_flags: u64::from_le_bytes(raw[SB_OFF_COMPAT_FLAGS..SB_OFF_COMPAT_FLAGS + 8].try_into().unwrap()),
        compat_ro_flags: u64::from_le_bytes(raw[SB_OFF_COMPAT_RO_FLAGS..SB_OFF_COMPAT_RO_FLAGS + 8].try_into().unwrap()),
        incompat_flags: u64::from_le_bytes(raw[SB_OFF_INCOMPAT_FLAGS..SB_OFF_INCOMPAT_FLAGS + 8].try_into().unwrap()),
        csum_type: u16::from_le_bytes(raw[SB_OFF_CSUM_TYPE..SB_OFF_CSUM_TYPE + 2].try_into().unwrap()),
        root_level: raw[SB_OFF_ROOT_LEVEL],
        chunk_root_level: raw[SB_OFF_CHUNK_ROOT_LEVEL],
        log_root_level: raw[SB_OFF_LOG_ROOT_LEVEL],
        dev_item,
        label,
        cache_generation: u64::from_le_bytes(raw[SB_OFF_CACHE_GENERATION..SB_OFF_CACHE_GENERATION + 8].try_into().unwrap()),
        uuid_tree_generation: u64::from_le_bytes(raw[SB_OFF_UUID_TREE_GENERATION..SB_OFF_UUID_TREE_GENERATION + 8].try_into().unwrap()),
        metadata_uuid,
        sys_chunk_array,
    })
}

/// Quick probe: read the superblock sector and check for Btrfs magic.
///
/// Returns `true` if the magic matches.  Does NOT validate checksums.
pub fn probe_btrfs_magic(disk: &dyn BlockDevice, partition_start_lba: u64) -> bool {
    let sb_sector = partition_start_lba + (BTRFS_SUPER_INFO_OFFSET / 512);
    let mut buf = [0u8; 512];
    if !disk.read_sectors(sb_sector, 1, &mut buf) {
        return false;
    }
    // Magic is at offset 0x40 within the superblock.  Since the superblock
    // starts at the beginning of the sector we just read, the magic is at
    // buf[0x40].  But wait — 0x10000 is exactly sector 128, and the superblock
    // is 4096 bytes = 8 sectors.  The magic at 0x40 falls inside the first
    // sector (sector 128), offset 0x40.  But our buf only has 512 bytes of
    // sector 128.  Offset 0x40 = 64, which is well within 512 bytes.
    let offset_in_sector = (BTRFS_SUPER_INFO_OFFSET % 512) as usize + SB_OFF_MAGIC;
    if offset_in_sector + 8 > 512 { return false; }
    let magic = u64::from_le_bytes(buf[offset_in_sector..offset_in_sector + 8].try_into().unwrap());
    magic == BTRFS_MAGIC
}
