// fs/btrfs/chunk.rs — Btrfs chunk tree: logical ↔ physical address mapping.
//
// Every Btrfs read/write operates on *logical* byte addresses.  The chunk tree
// maps these to physical byte offsets on the underlying block device.
//
// Bootstrap sequence:
//   1. Parse sys_chunk_array from the superblock (inline chunk entries that
//      let us find the chunk tree itself).
//   2. Read the full chunk tree using the bootstrap mapping.
//   3. Merge all discovered chunks into the final mapping table.
//
// For single-device volumes (our only supported configuration), each chunk
// has exactly one stripe with a 1:1 logical → physical mapping.
//
// Reference: btrfs on-disk format §4 "Chunk Tree".

extern crate alloc;

use alloc::vec::Vec;
use super::ondisk::*;

/// A single mapping entry: [logical_start, logical_start + length) → physical.
#[derive(Clone, Debug)]
pub struct ChunkMapping {
    /// Logical start address.
    pub logical: u64,
    /// Length of the chunk in bytes.
    pub length: u64,
    /// Physical byte offset on the device.
    pub physical: u64,
    /// Device ID (always 1 for single-device).
    pub devid: u64,
    /// Chunk type flags (DATA, METADATA, SYSTEM).
    pub chunk_type: u64,
}

/// The complete logical → physical mapping table.
#[derive(Clone)]
pub struct ChunkMap {
    /// Sorted by logical address (ascending).
    entries: Vec<ChunkMapping>,
}

impl ChunkMap {
    /// Create a new empty chunk map.
    pub fn new() -> Self {
        ChunkMap { entries: Vec::new() }
    }

    /// Insert a chunk mapping, keeping the table sorted by logical address.
    pub fn insert(&mut self, mapping: ChunkMapping) {
        // Check for duplicate (same logical start).
        for existing in self.entries.iter_mut() {
            if existing.logical == mapping.logical {
                // Update in place (chunk tree entry supersedes bootstrap).
                *existing = mapping;
                return;
            }
        }
        // Insert sorted.
        let pos = self.entries.partition_point(|e| e.logical < mapping.logical);
        self.entries.insert(pos, mapping);
    }

    /// Translate a logical byte address to a physical byte address.
    ///
    /// Returns `None` if the address doesn't fall inside any known chunk.
    pub fn logical_to_physical(&self, logical: u64) -> Option<u64> {
        // Binary search for the chunk that contains `logical`.
        let idx = match self.entries.binary_search_by(|e| e.logical.cmp(&logical)) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let chunk = &self.entries[idx];
        let offset_in_chunk = logical - chunk.logical;
        if offset_in_chunk >= chunk.length {
            return None;
        }
        Some(chunk.physical + offset_in_chunk)
    }

    /// Return the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterate over all chunks.
    pub fn iter(&self) -> impl Iterator<Item = &ChunkMapping> {
        self.entries.iter()
    }

    /// Find a free region of at least `size` bytes within chunks of the given
    /// type.  Returns the logical address of the start of the free region.
    ///
    /// This is a simple linear scan — not production-grade, but correct for
    /// our single-device use case.  A real implementation would use the free
    /// space tree or extent tree.
    pub fn find_chunk_of_type(&self, chunk_type_mask: u64) -> Option<&ChunkMapping> {
        self.entries.iter().find(|e| (e.chunk_type & chunk_type_mask) != 0)
    }
}

/// Parse the superblock's `sys_chunk_array` to bootstrap chunk mappings.
///
/// The sys_chunk_array is a packed sequence of (key, chunk_item, stripes).
/// Each key has type CHUNK_ITEM_KEY and objectid FIRST_CHUNK_TREE_OBJECTID.
///
/// Returns the bootstrapped `ChunkMap`.
pub fn parse_sys_chunk_array(
    sys_chunk_array: &[u8],
    sys_chunk_array_size: u32,
) -> ChunkMap {
    let mut map = ChunkMap::new();
    let total = sys_chunk_array_size as usize;
    let mut offset = 0;

    while offset + BtrfsKey::SIZE + BtrfsChunkItem::SIZE <= total {
        let key = BtrfsKey::from_bytes(&sys_chunk_array[offset..]);
        offset += BtrfsKey::SIZE;

        let chunk = BtrfsChunkItem::from_bytes(&sys_chunk_array[offset..]);
        offset += BtrfsChunkItem::SIZE;

        // Read the first stripe (single-device: num_stripes == 1).
        let num_stripes = chunk.num_stripes.max(1) as usize;
        if offset + BtrfsStripe::SIZE > total {
            break;
        }
        let stripe = BtrfsStripe::from_bytes(&sys_chunk_array[offset..]);
        offset += num_stripes * BtrfsStripe::SIZE;

        map.insert(ChunkMapping {
            logical: key.offset,
            length: chunk.length,
            physical: stripe.offset,
            devid: stripe.devid,
            chunk_type: chunk.chunk_type,
        });
    }

    map
}

/// Read the full chunk tree and merge its entries into the map.
///
/// Uses `read_node_fn` to read a B-tree node at a given logical address.
/// This avoids a circular dependency on btree.rs — the caller provides
/// a closure that handles the actual I/O + chunk mapping.
pub fn load_chunk_tree(
    map: &mut ChunkMap,
    chunk_root_logical: u64,
    chunk_root_level: u8,
    nodesize: u32,
    read_node_fn: &dyn Fn(u64) -> Option<Vec<u8>>,
) {
    load_chunk_tree_recursive(map, chunk_root_logical, chunk_root_level, nodesize, read_node_fn);
}

fn load_chunk_tree_recursive(
    map: &mut ChunkMap,
    node_logical: u64,
    level: u8,
    nodesize: u32,
    read_node_fn: &dyn Fn(u64) -> Option<Vec<u8>>,
) {
    let node_data = match read_node_fn(node_logical) {
        Some(data) => data,
        None => return,
    };

    let header = BtrfsHeader::from_bytes(&node_data);
    let nritems = header.nritems as usize;

    if level == 0 {
        // Leaf node: parse chunk items.
        for i in 0..nritems {
            let item_offset = BtrfsHeader::SIZE + i * BtrfsItem::SIZE;
            if item_offset + BtrfsItem::SIZE > node_data.len() { break; }
            let item = BtrfsItem::from_bytes(&node_data[item_offset..]);

            if item.key.item_type != BTRFS_CHUNK_ITEM_KEY { continue; }

            let data_offset = item.offset as usize;
            let data_size = item.size as usize;
            if data_offset + data_size > nodesize as usize { continue; }
            if data_size < BtrfsChunkItem::SIZE + BtrfsStripe::SIZE { continue; }

            let chunk = BtrfsChunkItem::from_bytes(&node_data[data_offset..]);
            let stripe = BtrfsStripe::from_bytes(&node_data[data_offset + BtrfsChunkItem::SIZE..]);

            map.insert(ChunkMapping {
                logical: item.key.offset,
                length: chunk.length,
                physical: stripe.offset,
                devid: stripe.devid,
                chunk_type: chunk.chunk_type,
            });
        }
    } else {
        // Internal node: recurse into children.
        for i in 0..nritems {
            let kp_offset = BtrfsHeader::SIZE + i * BtrfsKeyPtr::SIZE;
            if kp_offset + BtrfsKeyPtr::SIZE > node_data.len() { break; }
            let kp = BtrfsKeyPtr::from_bytes(&node_data[kp_offset..]);
            load_chunk_tree_recursive(map, kp.blockptr, level - 1, nodesize, read_node_fn);
        }
    }
}
