// fs/partition.rs — MBR and GPT partition table parsing.
//
// Reads sector 0 of a block device, determines the partition scheme, and
// returns a list of discovered partitions.
//
// Supported schemes:
//   - MBR (Master Boot Record): signature 0x55AA at bytes [510..512].
//   - GPT: MBR entry type 0xEE at partition entry 0 → GPT protective MBR.
//   - Bare (no partition table): MBR signature absent → one partition
//     covering the entire device (type 0xFF).
//
// Device naming convention (matches Bazzulto sbd* scheme):
//   sbd0 = first block device, sbd1 = second, etc.
//   Mount points: /mnt/sbd0, /mnt/sbd1, ...
//
// Reference:
//   MBR: BIOS Enhanced Disk Drive Specification (EDD-3, 1999).
//   GPT: UEFI Specification 2.10, §5.3.
//   FAT32 partition types: 0x0B (CHS), 0x0C (LBA).

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::hal::disk::BlockDevice;

// ---------------------------------------------------------------------------
// Partition
// ---------------------------------------------------------------------------

/// A discovered disk partition.
#[derive(Clone)]
pub struct Partition {
    /// Block device that contains this partition.
    pub disk: Arc<dyn BlockDevice>,
    /// Index of the disk in the global disk registry (0 = sbd0, 1 = sbd1, …).
    pub disk_index: usize,
    /// Index of this partition on the disk (0-based).
    pub part_index: usize,
    /// First logical block address.
    pub start_lba: u64,
    /// Size in sectors.
    pub sector_count: u64,
    /// MBR partition type byte.
    ///
    /// Common values: 0x0B/0x0C = FAT32, 0xEE = GPT protective, 0xFF = bare.
    pub partition_type: u8,
}

impl Partition {
    /// Return `true` if this partition is a FAT32 candidate.
    ///
    /// Types 0x0B (FAT32 CHS) and 0x0C (FAT32 LBA) are definitive.
    /// Type 0xFF (bare disk sentinel) requires BPB probing.
    pub fn is_fat32_candidate(&self) -> bool {
        matches!(self.partition_type, 0x0B | 0x0C | 0xFF)
    }

    /// Mount point for this partition.
    ///
    /// Convention: `/mnt/disk{letter}{partition_number}`
    ///   - letter:           disk index as lowercase ASCII (0 → 'a', 1 → 'b', …)
    ///   - partition_number: partition index + 1 (0-based index → 1-based number)
    ///
    /// Examples:
    ///   disk 0, partition 0 → `/mnt/diska1`
    ///   disk 0, partition 1 → `/mnt/diska2`
    ///   disk 1, partition 0 → `/mnt/diskb1`
    ///   disk 2, partition 0 → `/mnt/diskc1`
    pub fn mount_path(&self) -> alloc::string::String {
        let disk_letter = (b'a' + (self.disk_index as u8).min(25)) as char;
        alloc::format!("/mnt/disk{}{}", disk_letter, self.part_index + 1)
    }
}

// ---------------------------------------------------------------------------
// MBR layout constants
// ---------------------------------------------------------------------------

const MBR_PARTITION_TABLE_OFFSET: usize = 446;
const MBR_ENTRY_SIZE:             usize = 16;
const MBR_ENTRY_COUNT:            usize = 4;
const MBR_SIGNATURE:              [u8; 2] = [0x55, 0xAA];

// Offsets within a single MBR partition entry.
const MBR_ENTRY_TYPE_OFFSET:  usize = 4;
const MBR_ENTRY_LBA_OFFSET:   usize = 8;
const MBR_ENTRY_COUNT_OFFSET: usize = 12;

const MBR_TYPE_GPT_PROTECTIVE: u8 = 0xEE;

// ---------------------------------------------------------------------------
// GPT layout constants
// ---------------------------------------------------------------------------

const GPT_HEADER_LBA:              u64    = 1;
const GPT_SIGNATURE:               &[u8; 8] = b"EFI PART";
const GPT_ENTRIES_START_LBA_OFFSET: usize = 72;
const GPT_NUM_ENTRIES_OFFSET:       usize = 80;
const GPT_ENTRY_SIZE_OFFSET:        usize = 84;

/// Microsoft Basic Data GUID (FAT32, exFAT, NTFS) stored little-endian.
///
/// GUID: {EBD0A0A2-B9E5-4433-87C0-68B6B72699C7}
const GPT_BASIC_DATA_GUID: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB,
    0xE5, 0xB9,
    0x33, 0x44,
    0x87, 0xC0,
    0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enumerate all partitions on `disk`.
///
/// `disk_index` is the index of `disk` in the global `DISK_REGISTRY`
/// (determines the `sbd{N}` name and mount path).
///
/// If the disk has no recognisable partition table, returns a single
/// `Partition` covering the entire device (type 0xFF).
pub fn enumerate_partitions(disk: Arc<dyn BlockDevice>, disk_index: usize) -> Vec<Partition> {
    let mut sector0 = [0u8; 512];
    if !disk.read_sectors(0, 1, &mut sector0) {
        return Vec::new();
    }

    if sector0[510..512] != MBR_SIGNATURE {
        // No recognisable partition table — whole disk as one partition.
        let sector_count = disk.sector_count();
        return alloc::vec![Partition {
            disk,
            disk_index,
            part_index: 0,
            start_lba: 0,
            sector_count,
            partition_type: 0xFF,
        }];
    }

    // Distinguish a real MBR from a FAT32 BPB written directly at sector 0
    // (bare/whole-disk FAT32 volume with no partition table).
    //
    // A FAT32 BPB always starts with a JMP SHORT (0xEB xx 0x90) or JMP NEAR
    // (0xE9 xx xx) instruction.  A real MBR bootstrap starts with 0xFA (CLI)
    // or other x86 prologue.  The signature 0x55AA is present in both, so the
    // byte-0 check is the reliable discriminator.
    //
    // Reference: FAT specification §3.1 (BS_jmpBoot field), MBR layout.
    let first_byte = sector0[0];
    let is_bpb_not_mbr = first_byte == 0xEB || first_byte == 0xE9;
    if is_bpb_not_mbr {
        // Bare FAT32 volume — treat as a single whole-disk partition.
        let sector_count = disk.sector_count();
        return alloc::vec![Partition {
            disk,
            disk_index,
            part_index: 0,
            start_lba: 0,
            sector_count,
            partition_type: 0x0C, // FAT32 LBA — known FAT32 volume
        }];
    }

    let entry0_type = sector0[MBR_PARTITION_TABLE_OFFSET + MBR_ENTRY_TYPE_OFFSET];
    if entry0_type == MBR_TYPE_GPT_PROTECTIVE {
        return parse_gpt(disk, disk_index);
    }

    parse_mbr(disk, disk_index, &sector0)
}

// ---------------------------------------------------------------------------
// MBR parser
// ---------------------------------------------------------------------------

fn parse_mbr(disk: Arc<dyn BlockDevice>, disk_index: usize, sector0: &[u8]) -> Vec<Partition> {
    let mut partitions: Vec<Partition> = Vec::new();

    for entry_index in 0..MBR_ENTRY_COUNT {
        let base = MBR_PARTITION_TABLE_OFFSET + entry_index * MBR_ENTRY_SIZE;
        let partition_type = sector0[base + MBR_ENTRY_TYPE_OFFSET];
        if partition_type == 0x00 { continue; }

        let start_lba = u32::from_le_bytes(
            sector0[base + MBR_ENTRY_LBA_OFFSET..base + MBR_ENTRY_LBA_OFFSET + 4]
                .try_into().unwrap_or([0; 4])
        ) as u64;
        let sector_count = u32::from_le_bytes(
            sector0[base + MBR_ENTRY_COUNT_OFFSET..base + MBR_ENTRY_COUNT_OFFSET + 4]
                .try_into().unwrap_or([0; 4])
        ) as u64;

        if start_lba == 0 || sector_count == 0 { continue; }

        partitions.push(Partition {
            disk: disk.clone(),
            disk_index,
            part_index: partitions.len(),
            start_lba,
            sector_count,
            partition_type,
        });
    }

    if partitions.is_empty() {
        // MBR signature present but all entries empty.
        let sector_count = disk.sector_count();
        partitions.push(Partition {
            disk,
            disk_index,
            part_index: 0,
            start_lba: 0,
            sector_count,
            partition_type: 0xFF,
        });
    }

    partitions
}

// ---------------------------------------------------------------------------
// GPT parser
// ---------------------------------------------------------------------------

fn parse_gpt(disk: Arc<dyn BlockDevice>, disk_index: usize) -> Vec<Partition> {
    let mut header = [0u8; 512];
    if !disk.read_sectors(GPT_HEADER_LBA, 1, &mut header) {
        return Vec::new();
    }
    if &header[0..8] != GPT_SIGNATURE { return Vec::new(); }

    let entries_start_lba = u64::from_le_bytes(
        header[GPT_ENTRIES_START_LBA_OFFSET..GPT_ENTRIES_START_LBA_OFFSET + 8]
            .try_into().unwrap_or([0; 8])
    );
    let num_entries = u32::from_le_bytes(
        header[GPT_NUM_ENTRIES_OFFSET..GPT_NUM_ENTRIES_OFFSET + 4]
            .try_into().unwrap_or([0; 4])
    ) as usize;
    let entry_size = u32::from_le_bytes(
        header[GPT_ENTRY_SIZE_OFFSET..GPT_ENTRY_SIZE_OFFSET + 4]
            .try_into().unwrap_or([0; 4])
    ) as usize;

    if entry_size == 0 || entry_size > 512 { return Vec::new(); }

    let entries_per_sector = 512 / entry_size;
    let sectors_needed = (num_entries + entries_per_sector - 1) / entries_per_sector;
    let mut buf = alloc::vec![0u8; sectors_needed * 512];
    if !disk.read_sectors(entries_start_lba, sectors_needed as u32, &mut buf) {
        return Vec::new();
    }

    let mut partitions: Vec<Partition> = Vec::new();

    for entry_index in 0..num_entries {
        let base = entry_index * entry_size;
        if base + entry_size > buf.len() { break; }

        let type_guid = &buf[base..base + 16];
        if type_guid.iter().all(|&b| b == 0) { continue; }

        let start_lba = u64::from_le_bytes(
            buf[base + 32..base + 40].try_into().unwrap_or([0; 8])
        );
        let end_lba = u64::from_le_bytes(
            buf[base + 40..base + 48].try_into().unwrap_or([0; 8])
        );
        if start_lba == 0 || end_lba < start_lba { continue; }

        let partition_type = if type_guid == GPT_BASIC_DATA_GUID { 0x0C } else { 0x01 };

        partitions.push(Partition {
            disk: disk.clone(),
            disk_index,
            part_index: partitions.len(),
            start_lba,
            sector_count: end_lba - start_lba + 1,
            partition_type,
        });
    }

    partitions
}
