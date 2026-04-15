// fs/btrfs/btree.rs — Btrfs B-tree search and traversal.
//
// Every Btrfs tree is a B-tree where internal nodes hold (key, blockptr)
// pairs and leaf nodes hold (key, offset, size) item descriptors whose data
// lives in the same leaf page.
//
// This module implements:
//   - `search_slot`: find the item (or insertion point) for a given key.
//   - `search_exact`: find an item with an exact key match.
//   - `search_first_ge`: find the first item whose key >= the search key.
//   - `iter_items_from`: iterate leaf items starting from a given slot.
//
// All operations work on a `BtrfsVolume` which provides the chunk map and
// disk I/O.
//
// Reference: btrfs on-disk format §3 "B-trees".

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::ondisk::*;
use super::chunk::ChunkMap;
use crate::hal::disk::BlockDevice;

/// A search result: the leaf node data and the slot index within it.
pub struct SearchResult {
    /// Raw bytes of the leaf node that contains (or would contain) the key.
    pub leaf: Vec<u8>,
    /// Slot index (0-based) within the leaf.
    ///
    /// If `exact` is true, `leaf[slot]` matches the search key exactly.
    /// If `exact` is false, `slot` is the insertion point (first key > search key).
    pub slot: usize,
    /// Whether the key was found exactly.
    pub exact: bool,
    /// Number of items in this leaf.
    pub nritems: usize,
}

/// Read a B-tree node from disk at the given logical address.
///
/// Returns `None` if the I/O fails or the address can't be mapped.
pub fn read_node(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    logical: u64,
    nodesize: u32,
) -> Option<Vec<u8>> {
    let physical = chunk_map.logical_to_physical(logical)?;
    let sector = partition_start_lba + physical / 512;
    let sectors = (nodesize as u64 + 511) / 512;
    let mut buf = vec![0u8; nodesize as usize];
    if !disk.read_sectors(sector, sectors as u32, &mut buf) {
        return None;
    }
    Some(buf)
}

/// Search for `key` in the B-tree rooted at `root_logical` with `root_level`.
///
/// Returns the leaf that contains the key (or where the key would be
/// inserted), plus the slot index and whether the match is exact.
pub fn search_slot(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    root_level: u8,
    key: &BtrfsKey,
) -> Option<SearchResult> {
    let mut current_logical = root_logical;
    let mut current_level = root_level;

    loop {
        let node = read_node(disk, partition_start_lba, chunk_map, current_logical, nodesize)?;
        let header = BtrfsHeader::from_bytes(&node);
        let nritems = header.nritems as usize;

        if current_level == 0 {
            // Leaf node — binary search for the key.
            let slot = binary_search_leaf(&node, nritems, key);
            let exact = slot < nritems && {
                let item = get_leaf_item(&node, slot);
                item.key == *key
            };
            return Some(SearchResult {
                leaf: node,
                slot,
                exact,
                nritems,
            });
        } else {
            // Internal node — find child to descend into.
            let child_idx = binary_search_internal(&node, nritems, key);
            let kp = get_keyptr(&node, child_idx);
            current_logical = kp.blockptr;
            current_level -= 1;
        }
    }
}

/// Search for exactly the given key.  Returns `None` if not found.
pub fn search_exact(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    root_level: u8,
    key: &BtrfsKey,
) -> Option<SearchResult> {
    let result = search_slot(disk, partition_start_lba, chunk_map, nodesize,
                             root_logical, root_level, key)?;
    if result.exact { Some(result) } else { None }
}

/// Search for the first item whose key >= `key`.
pub fn search_first_ge(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    root_level: u8,
    key: &BtrfsKey,
) -> Option<SearchResult> {
    let result = search_slot(disk, partition_start_lba, chunk_map, nodesize,
                             root_logical, root_level, key)?;
    if result.slot < result.nritems {
        Some(result)
    } else {
        None // key is beyond all items in the tree
    }
}

/// Get the raw item data for slot `slot` in a leaf node.
///
/// Returns `(key, data_slice)` where `data_slice` is a sub-slice of `leaf`.
pub fn get_item_data<'a>(leaf: &'a [u8], slot: usize) -> Option<(BtrfsKey, &'a [u8])> {
    let item = get_leaf_item(leaf, slot);
    let data_start = item.offset as usize;
    let data_end = data_start + item.size as usize;
    if data_end > leaf.len() { return None; }
    Some((item.key, &leaf[data_start..data_end]))
}

/// Iterator over consecutive leaf items starting from `start_slot`.
///
/// Yields `(key, data)` for each item.  Does NOT cross to the next leaf —
/// callers must handle leaf boundaries separately if needed.
pub fn iter_items_from(leaf: &[u8], start_slot: usize, nritems: usize)
    -> impl Iterator<Item = (BtrfsKey, &[u8])>
{
    (start_slot..nritems).filter_map(move |i| get_item_data(leaf, i))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the `BtrfsItem` at slot `index` in a leaf node.
fn get_leaf_item(node: &[u8], index: usize) -> BtrfsItem {
    let offset = BtrfsHeader::SIZE + index * BtrfsItem::SIZE;
    BtrfsItem::from_bytes(&node[offset..])
}

/// Read the `BtrfsKeyPtr` at slot `index` in an internal node.
fn get_keyptr(node: &[u8], index: usize) -> BtrfsKeyPtr {
    let offset = BtrfsHeader::SIZE + index * BtrfsKeyPtr::SIZE;
    BtrfsKeyPtr::from_bytes(&node[offset..])
}

/// Binary search in a leaf node for `key`.
///
/// Returns the index of the matching item, or the index where the key would
/// be inserted (first item with key > search key).
fn binary_search_leaf(node: &[u8], nritems: usize, key: &BtrfsKey) -> usize {
    if nritems == 0 { return 0; }
    let mut lo: usize = 0;
    let mut hi: usize = nritems;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let item = get_leaf_item(node, mid);
        match item.key.cmp(key) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Equal => return mid,
            core::cmp::Ordering::Greater => hi = mid,
        }
    }
    lo
}

/// Binary search in an internal node for the child that may contain `key`.
///
/// Returns the index of the child to descend into.  For an internal node
/// with N key/pointer pairs, key[i] is the *first* key in child[i].
/// We want the highest i where key[i] <= search_key.
fn binary_search_internal(node: &[u8], nritems: usize, key: &BtrfsKey) -> usize {
    if nritems <= 1 { return 0; }
    let mut lo: usize = 0;
    let mut hi: usize = nritems;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let kp = get_keyptr(node, mid);
        if kp.key <= *key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // lo is now the first slot with key > search key.
    // We want the slot before that (the last slot with key <= search key).
    if lo > 0 { lo - 1 } else { 0 }
}
