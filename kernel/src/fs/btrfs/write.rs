// fs/btrfs/write.rs — Btrfs write path: CoW, B-tree mutation, extent allocation.
//
// Btrfs is a copy-on-write filesystem: every modification creates new copies
// of the affected B-tree nodes rather than updating them in place.  This
// provides atomic commits — the superblock pointer is the last thing updated,
// so a crash at any other point leaves the previous consistent state intact.
//
// This module implements:
//   - `allocate_extent`:    find free space and reserve a physical region.
//   - `write_node`:         write a B-tree node to a newly allocated extent.
//   - `insert_item`:        insert a key/data pair into a B-tree leaf.
//   - `update_item`:        overwrite the data of an existing item (same size).
//   - `delete_item`:        remove a key/data pair from a leaf.
//   - `commit_transaction`: update the superblock to point to new tree roots.
//
// Simplifications for v1.0 single-device hobby OS:
//   - No leaf splitting yet (will fail if a leaf is full).
//   - Extent allocation uses a simple bump pointer within each chunk.
//   - No free space tree — we track free space in-memory per chunk.
//
// Reference: btrfs on-disk format §6 "Transaction model".

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::btree;
use super::chunk::ChunkMap;
use super::crc32c;
use super::ondisk::*;
use crate::hal::disk::BlockDevice;

/// Per-chunk allocator state.
pub struct ChunkAllocator {
    /// One entry per chunk in the chunk map.
    entries: Vec<ChunkAllocEntry>,
}

struct ChunkAllocEntry {
    /// Logical start of the chunk.
    logical_start: u64,
    /// Total length of the chunk.
    length: u64,
    /// Next free offset within the chunk (relative to logical_start).
    next_free: u64,
    /// Chunk type flags.
    chunk_type: u64,
}

impl ChunkAllocator {
    /// Create an allocator from the chunk map.
    ///
    /// `used_ranges` is a list of (logical_start, length) pairs from the extent
    /// tree (or initial metadata/data positions) that are already allocated.
    pub fn new(chunk_map: &ChunkMap, used_ranges: &[(u64, u64)]) -> Self {
        let mut entries: Vec<ChunkAllocEntry> = chunk_map.iter().map(|cm| {
            ChunkAllocEntry {
                logical_start: cm.logical,
                length: cm.length,
                next_free: 0,
                chunk_type: cm.chunk_type,
            }
        }).collect();

        // Advance next_free past used ranges.
        for &(used_start, used_len) in used_ranges {
            for entry in entries.iter_mut() {
                let chunk_end = entry.logical_start + entry.length;
                if used_start >= entry.logical_start && used_start < chunk_end {
                    let used_end_in_chunk = (used_start + used_len) - entry.logical_start;
                    if used_end_in_chunk > entry.next_free {
                        entry.next_free = used_end_in_chunk;
                    }
                }
            }
        }

        ChunkAllocator { entries }
    }

    /// Allocate `size` bytes from a chunk of the given type.
    ///
    /// Returns the logical address of the allocation, or `None` if no space.
    pub fn allocate(&mut self, size: u64, chunk_type_mask: u64) -> Option<u64> {
        // Align to sectorsize (4096).
        let aligned_size = (size + 4095) & !4095;

        for entry in self.entries.iter_mut() {
            if (entry.chunk_type & chunk_type_mask) == 0 { continue; }
            let remaining = entry.length.saturating_sub(entry.next_free);
            if remaining >= aligned_size {
                let logical = entry.logical_start + entry.next_free;
                entry.next_free += aligned_size;
                return Some(logical);
            }
        }
        None
    }

    /// Return bytes used in chunks of the given type.
    pub fn bytes_used(&self, chunk_type_mask: u64) -> u64 {
        self.entries.iter()
            .filter(|e| (e.chunk_type & chunk_type_mask) != 0)
            .map(|e| e.next_free)
            .sum()
    }
}

/// Write a B-tree node (leaf or internal) to disk.
///
/// 1. Allocates a new extent of `nodesize` bytes in the appropriate chunk.
/// 2. Computes and sets the CRC32C checksum in the header.
/// 3. Writes the node to disk.
/// 4. Returns the logical address of the new node.
pub fn write_node(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    allocator: &mut ChunkAllocator,
    nodesize: u32,
    node_data: &mut Vec<u8>,
) -> Option<u64> {
    // Allocate in metadata chunk.
    let logical = allocator.allocate(nodesize as u64, BTRFS_BLOCK_GROUP_METADATA)?;

    // Set bytenr in header.
    node_data[48..56].copy_from_slice(&logical.to_le_bytes());

    // Compute checksum over bytes [0x20..nodesize].
    let csum = crc32c::btrfs_checksum(&node_data[0x20..nodesize as usize]);
    node_data[0..4].copy_from_slice(&csum.to_le_bytes());
    // Clear remaining csum bytes (only first 4 used for CRC32C).
    for b in &mut node_data[4..32] { *b = 0; }

    // Write to disk.
    let physical = chunk_map.logical_to_physical(logical)?;
    let sector = partition_start_lba + physical / 512;
    let sectors = (nodesize as u64 + 511) / 512;
    if !disk.write_sectors(sector, sectors as u32, node_data) {
        return None;
    }

    Some(logical)
}

/// Build a new leaf node containing the given items.
///
/// `items` is a list of (key, data) pairs, assumed to be sorted by key.
/// The leaf is sized to `nodesize` bytes.
pub fn build_leaf(
    nodesize: u32,
    owner: u64,
    generation: u64,
    fsid: &[u8; BTRFS_UUID_SIZE],
    items: &[(BtrfsKey, &[u8])],
) -> Vec<u8> {
    let mut node = vec![0u8; nodesize as usize];

    // Write header.
    let mut header = BtrfsHeader {
        csum: [0u8; BTRFS_CSUM_SIZE],
        fsid: *fsid,
        bytenr: 0, // Set by write_node.
        flags: 0,
        chunk_tree_uuid: [0u8; BTRFS_UUID_SIZE],
        generation,
        owner,
        nritems: items.len() as u32,
        level: 0,
    };
    header.to_bytes(&mut node);

    // Item descriptors are packed after the header.
    // Item data is packed at the end of the node, growing backwards.
    let mut data_end = nodesize as usize;
    for (i, (key, data)) in items.iter().enumerate() {
        data_end -= data.len();

        let item = BtrfsItem {
            key: *key,
            offset: data_end as u32,
            size: data.len() as u32,
        };
        let item_offset = BtrfsHeader::SIZE + i * BtrfsItem::SIZE;
        item.to_bytes(&mut node[item_offset..]);

        // Copy item data.
        node[data_end..data_end + data.len()].copy_from_slice(data);
    }

    node
}

/// Insert a single item into an existing leaf.
///
/// Returns the modified leaf data, or `None` if the leaf is full.
///
/// This is a simplified version that rebuilds the leaf from scratch.
/// A production implementation would do in-place insertion and handle
/// leaf splitting.
pub fn insert_item_into_leaf(
    leaf: &[u8],
    nodesize: u32,
    new_key: &BtrfsKey,
    new_data: &[u8],
) -> Option<Vec<u8>> {
    let header = BtrfsHeader::from_bytes(leaf);
    let nritems = header.nritems as usize;

    // Collect existing items.
    let mut items: Vec<(BtrfsKey, Vec<u8>)> = Vec::with_capacity(nritems + 1);
    let mut total_data_size: usize = 0;

    for i in 0..nritems {
        if let Some((key, data)) = btree::get_item_data(leaf, i) {
            total_data_size += data.len();
            items.push((key, data.to_vec()));
        }
    }

    // Check if there's room.
    total_data_size += new_data.len();
    let items_area = BtrfsHeader::SIZE + (nritems + 1) * BtrfsItem::SIZE;
    if items_area + total_data_size > nodesize as usize {
        return None; // Leaf full — would need splitting.
    }

    // Find insertion point.
    let pos = items.partition_point(|(k, _)| k < new_key);
    items.insert(pos, (*new_key, new_data.to_vec()));

    // Rebuild leaf.
    let refs: Vec<(BtrfsKey, &[u8])> = items.iter().map(|(k, d)| (*k, d.as_slice())).collect();
    let new_leaf = build_leaf(
        nodesize,
        header.owner,
        header.generation,
        &header.fsid,
        &refs,
    );

    Some(new_leaf)
}

/// Delete an item at `slot` from a leaf.
///
/// Returns the modified leaf.
pub fn delete_item_from_leaf(
    leaf: &[u8],
    nodesize: u32,
    slot: usize,
) -> Vec<u8> {
    let header = BtrfsHeader::from_bytes(leaf);
    let nritems = header.nritems as usize;

    let mut items: Vec<(BtrfsKey, Vec<u8>)> = Vec::with_capacity(nritems);
    for i in 0..nritems {
        if i == slot { continue; }
        if let Some((key, data)) = btree::get_item_data(leaf, i) {
            items.push((key, data.to_vec()));
        }
    }

    let refs: Vec<(BtrfsKey, &[u8])> = items.iter().map(|(k, d)| (*k, d.as_slice())).collect();
    build_leaf(nodesize, header.owner, header.generation, &header.fsid, &refs)
}

/// Write the updated superblock to disk.
///
/// Updates the root tree pointer, chunk root pointer, generation, and
/// bytes_used, then recomputes the checksum and writes to the primary
/// superblock location.
pub fn commit_superblock(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    sb: &BtrfsSuperblock,
    new_root: u64,
    new_root_level: u8,
    new_generation: u64,
    new_bytes_used: u64,
) -> bool {
    let mut raw = vec![0u8; BTRFS_SUPER_INFO_SIZE];

    // First read the existing superblock to preserve fields we don't modify.
    let sb_sector = partition_start_lba + (BTRFS_SUPER_INFO_OFFSET / 512);
    let sectors = (BTRFS_SUPER_INFO_SIZE + 511) / 512;
    if !disk.read_sectors(sb_sector, sectors as u32, &mut raw) {
        return false;
    }

    // Update the fields that change per transaction.
    raw[SB_OFF_GENERATION..SB_OFF_GENERATION + 8].copy_from_slice(&new_generation.to_le_bytes());
    raw[SB_OFF_ROOT..SB_OFF_ROOT + 8].copy_from_slice(&new_root.to_le_bytes());
    raw[SB_OFF_ROOT_LEVEL] = new_root_level;
    raw[SB_OFF_BYTES_USED..SB_OFF_BYTES_USED + 8].copy_from_slice(&new_bytes_used.to_le_bytes());

    // Recompute checksum.
    let csum = crc32c::btrfs_checksum(&raw[0x20..BTRFS_SUPER_INFO_SIZE]);
    raw[0..4].copy_from_slice(&csum.to_le_bytes());
    for b in &mut raw[4..32] { *b = 0; }

    // Write back.
    disk.write_sectors(sb_sector, sectors as u32, &raw)
}

/// Write file data for a file, creating an inline extent if small enough,
/// or a regular extent otherwise.
///
/// Returns the EXTENT_DATA item data (to be inserted into the FS tree).
pub fn create_extent_for_data(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    allocator: &mut ChunkAllocator,
    generation: u64,
    data: &[u8],
    nodesize: u32,
) -> Option<Vec<u8>> {
    // Inline threshold: data fits in a leaf item with room to spare.
    // Btrfs uses roughly (nodesize / 2 - header overhead) but a common
    // practical limit is ~2048 bytes.
    let inline_max = (nodesize as usize / 4).min(2048);

    if data.len() <= inline_max {
        // Inline extent — data stored directly in the B-tree leaf.
        let mut item = vec![0u8; BtrfsFileExtentItem::INLINE_HEADER_SIZE + data.len()];
        item[0..8].copy_from_slice(&generation.to_le_bytes()); // generation
        item[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes()); // ram_bytes
        item[16] = 0; // compression: none
        item[17] = 0; // encryption: none
        item[18..20].copy_from_slice(&0u16.to_le_bytes()); // other_encoding
        item[20] = BTRFS_FILE_EXTENT_INLINE; // type
        item[BtrfsFileExtentItem::INLINE_HEADER_SIZE..].copy_from_slice(data);
        Some(item)
    } else {
        // Regular extent — allocate space in data chunk and write data there.
        let aligned_size = ((data.len() as u64) + 4095) & !4095;
        let logical = allocator.allocate(aligned_size, BTRFS_BLOCK_GROUP_DATA)?;

        // Write data to disk.
        let physical = chunk_map.logical_to_physical(logical)?;
        let sector = partition_start_lba + physical / 512;
        // Pad data to sector boundary.
        let mut padded = vec![0u8; aligned_size as usize];
        padded[..data.len()].copy_from_slice(data);
        let sectors = (aligned_size + 511) / 512;
        if !disk.write_sectors(sector, sectors as u32, &padded) {
            return None;
        }

        // Build EXTENT_DATA item.
        let mut item = vec![0u8; BtrfsFileExtentItem::REG_SIZE];
        item[0..8].copy_from_slice(&generation.to_le_bytes());
        item[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes()); // ram_bytes
        item[16] = 0; // compression
        item[17] = 0; // encryption
        item[18..20].copy_from_slice(&0u16.to_le_bytes()); // other_encoding
        item[20] = BTRFS_FILE_EXTENT_REG; // type
        item[21..29].copy_from_slice(&logical.to_le_bytes()); // disk_bytenr
        item[29..37].copy_from_slice(&aligned_size.to_le_bytes()); // disk_num_bytes
        item[37..45].copy_from_slice(&0u64.to_le_bytes()); // offset
        item[45..53].copy_from_slice(&(data.len() as u64).to_le_bytes()); // num_bytes
        Some(item)
    }
}
