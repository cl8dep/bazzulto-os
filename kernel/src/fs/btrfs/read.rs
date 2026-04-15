// fs/btrfs/read.rs — Btrfs read path: inode, directory, and file data.
//
// Provides high-level operations on the FS tree:
//   - `read_inode`:    fetch an inode item by objectid.
//   - `lookup_dir`:    look up a name in a directory (DIR_ITEM hash lookup).
//   - `readdir_index`: iterate directory entries by sequence (DIR_INDEX).
//   - `read_file`:     read file data using extent items.
//
// All operations delegate to btree.rs for B-tree search and use the chunk map
// for address translation.
//
// Reference: btrfs on-disk format §5 "FS Tree".

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::btree;
use super::chunk::ChunkMap;
use super::ondisk::*;
use crate::hal::disk::BlockDevice;

/// A resolved directory entry (from lookup or readdir).
#[derive(Clone, Debug)]
pub struct BtrfsDirEntry {
    /// Name of the entry.
    pub name: String,
    /// Objectid of the child inode.
    pub objectid: u64,
    /// File type (BTRFS_FT_*).
    pub file_type: u8,
}

/// Read an inode item from the FS tree.
pub fn read_inode(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_root_logical: u64,
    fs_root_level: u8,
    objectid: u64,
) -> Option<BtrfsInodeItem> {
    let key = BtrfsKey::new(objectid, BTRFS_INODE_ITEM_KEY, 0);
    let result = btree::search_exact(
        disk, partition_start_lba, chunk_map, nodesize,
        fs_root_logical, fs_root_level, &key,
    )?;
    let (_k, data) = btree::get_item_data(&result.leaf, result.slot)?;
    if data.len() < BtrfsInodeItem::SIZE { return None; }
    Some(BtrfsInodeItem::from_bytes(data))
}

/// Look up a name in a directory using DIR_ITEM (hash-based lookup).
///
/// Returns the child objectid and file type on success.
pub fn lookup_dir(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_root_logical: u64,
    fs_root_level: u8,
    dir_objectid: u64,
    name: &str,
) -> Option<BtrfsDirEntry> {
    let name_hash = btrfs_name_hash(name.as_bytes());
    let key = BtrfsKey::new(dir_objectid, BTRFS_DIR_ITEM_KEY, name_hash);
    let result = btree::search_exact(
        disk, partition_start_lba, chunk_map, nodesize,
        fs_root_logical, fs_root_level, &key,
    )?;

    let (_k, data) = btree::get_item_data(&result.leaf, result.slot)?;

    // A DIR_ITEM may contain multiple entries with the same hash (collision).
    // Walk the packed entries until we find the name match.
    let mut offset = 0;
    while offset + BtrfsDirItem::HEADER_SIZE <= data.len() {
        let dir_item = BtrfsDirItem::from_bytes(&data[offset..]);
        let name_start = offset + BtrfsDirItem::HEADER_SIZE;
        let name_end = name_start + dir_item.name_len as usize;
        if name_end > data.len() { break; }

        let entry_name = &data[name_start..name_end];
        if entry_name == name.as_bytes() {
            let entry_name_str = core::str::from_utf8(entry_name).ok()?;
            return Some(BtrfsDirEntry {
                name: String::from(entry_name_str),
                objectid: dir_item.location.objectid,
                file_type: dir_item.dir_type,
            });
        }

        // Advance past this entry's header + name + data.
        offset = name_end + dir_item.data_len as usize;
    }

    None
}

/// Read directory entries by index (for readdir / getdents64).
///
/// Returns the `index`-th entry (0-based) in the directory, using DIR_INDEX
/// items which are ordered by sequence number.
pub fn readdir_index(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_root_logical: u64,
    fs_root_level: u8,
    dir_objectid: u64,
    index: usize,
) -> Option<BtrfsDirEntry> {
    // DIR_INDEX items use the directory objectid and a monotonically
    // increasing sequence number as the key offset.  The first entry is
    // typically at offset 2 (skipping . and ..).
    //
    // Strategy: search for the first DIR_INDEX item in this directory,
    // then advance `index` times.

    let search_key = BtrfsKey::new(dir_objectid, BTRFS_DIR_INDEX_KEY, 0);
    let result = btree::search_first_ge(
        disk, partition_start_lba, chunk_map, nodesize,
        fs_root_logical, fs_root_level, &search_key,
    )?;

    let mut count = 0;
    for (key, data) in btree::iter_items_from(&result.leaf, result.slot, result.nritems) {
        if key.objectid != dir_objectid || key.item_type != BTRFS_DIR_INDEX_KEY {
            break;
        }
        if count == index {
            return parse_dir_item_entry(data);
        }
        count += 1;
    }

    None
}

/// Read file data from extent items.
///
/// Reads up to `buf.len()` bytes starting at file offset `file_offset`.
/// Returns the number of bytes actually read.
pub fn read_file(
    disk: &dyn BlockDevice,
    partition_start_lba: u64,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_root_logical: u64,
    fs_root_level: u8,
    file_objectid: u64,
    file_offset: u64,
    buf: &mut [u8],
) -> usize {
    if buf.is_empty() { return 0; }

    // Search for EXTENT_DATA items covering the requested range.
    // EXTENT_DATA key offset = file offset where this extent starts.
    let search_key = BtrfsKey::new(file_objectid, BTRFS_EXTENT_DATA_KEY, 0);
    let result = match btree::search_first_ge(
        disk, partition_start_lba, chunk_map, nodesize,
        fs_root_logical, fs_root_level, &search_key,
    ) {
        Some(r) => r,
        None => return 0,
    };

    let mut bytes_read: usize = 0;
    let end_offset = file_offset + buf.len() as u64;

    for (key, data) in btree::iter_items_from(&result.leaf, result.slot, result.nritems) {
        if key.objectid != file_objectid || key.item_type != BTRFS_EXTENT_DATA_KEY {
            break;
        }
        if bytes_read >= buf.len() { break; }

        let extent_file_offset = key.offset;
        if data.len() < BtrfsFileExtentItem::INLINE_HEADER_SIZE { continue; }

        let extent = BtrfsFileExtentItem::from_bytes(data);

        match extent.extent_type {
            BTRFS_FILE_EXTENT_INLINE => {
                // Inline data follows the header directly in the item.
                let inline_data = &data[BtrfsFileExtentItem::INLINE_HEADER_SIZE..];
                let extent_end = extent_file_offset + inline_data.len() as u64;

                if file_offset >= extent_end || end_offset <= extent_file_offset {
                    continue; // No overlap.
                }

                let src_start = if file_offset > extent_file_offset {
                    (file_offset - extent_file_offset) as usize
                } else {
                    0
                };
                let dst_start = if extent_file_offset > file_offset {
                    (extent_file_offset - file_offset) as usize
                } else {
                    0
                };
                let copy_len = inline_data.len().saturating_sub(src_start)
                    .min(buf.len().saturating_sub(dst_start));
                if copy_len > 0 {
                    buf[dst_start..dst_start + copy_len]
                        .copy_from_slice(&inline_data[src_start..src_start + copy_len]);
                    bytes_read = (dst_start + copy_len).max(bytes_read);
                }
            }
            BTRFS_FILE_EXTENT_REG | BTRFS_FILE_EXTENT_PREALLOC => {
                if extent.disk_bytenr == 0 {
                    // Hole — fill with zeros (already zero in caller's buffer).
                    let hole_end = extent_file_offset + extent.num_bytes;
                    if file_offset < hole_end && end_offset > extent_file_offset {
                        let dst_start = if extent_file_offset > file_offset {
                            (extent_file_offset - file_offset) as usize
                        } else {
                            0
                        };
                        let dst_end = ((hole_end - file_offset) as usize).min(buf.len());
                        for b in &mut buf[dst_start..dst_end] { *b = 0; }
                        bytes_read = dst_end.max(bytes_read);
                    }
                    continue;
                }

                let extent_end = extent_file_offset + extent.num_bytes;
                if file_offset >= extent_end || end_offset <= extent_file_offset {
                    continue; // No overlap.
                }

                // Calculate the disk read range.
                let read_start_in_extent = if file_offset > extent_file_offset {
                    file_offset - extent_file_offset
                } else {
                    0
                };
                let read_end_in_extent = if end_offset < extent_end {
                    end_offset - extent_file_offset
                } else {
                    extent.num_bytes
                };
                let disk_logical = extent.disk_bytenr + extent.offset + read_start_in_extent;
                let read_len = (read_end_in_extent - read_start_in_extent) as usize;

                let physical = match chunk_map.logical_to_physical(disk_logical) {
                    Some(p) => p,
                    None => continue,
                };

                let sector = partition_start_lba + physical / 512;
                let sector_offset = (physical % 512) as usize;

                // Read full sectors, then copy the relevant portion.
                let total_bytes = sector_offset + read_len;
                let sectors_needed = (total_bytes + 511) / 512;
                let mut sector_buf = vec![0u8; sectors_needed * 512];
                if !disk.read_sectors(sector, sectors_needed as u32, &mut sector_buf) {
                    continue;
                }

                let dst_start = if extent_file_offset > file_offset {
                    (extent_file_offset - file_offset) as usize + read_start_in_extent as usize
                } else {
                    (read_start_in_extent - (file_offset - extent_file_offset).min(read_start_in_extent)) as usize
                };
                // Simplify: dst_start is where in buf[] we write.
                let dst_start = if file_offset >= extent_file_offset {
                    0 + bytes_read.max(
                        (extent_file_offset + read_start_in_extent).saturating_sub(file_offset) as usize
                    )
                } else {
                    (extent_file_offset + read_start_in_extent - file_offset) as usize
                };

                let copy_len = read_len.min(buf.len().saturating_sub(dst_start));
                if copy_len > 0 {
                    buf[dst_start..dst_start + copy_len]
                        .copy_from_slice(&sector_buf[sector_offset..sector_offset + copy_len]);
                    bytes_read = (dst_start + copy_len).max(bytes_read);
                }
            }
            _ => {} // Unknown extent type, skip.
        }
    }

    bytes_read
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a single dir item entry from raw item data.
fn parse_dir_item_entry(data: &[u8]) -> Option<BtrfsDirEntry> {
    if data.len() < BtrfsDirItem::HEADER_SIZE { return None; }
    let dir_item = BtrfsDirItem::from_bytes(data);
    let name_start = BtrfsDirItem::HEADER_SIZE;
    let name_end = name_start + dir_item.name_len as usize;
    if name_end > data.len() { return None; }
    let name = core::str::from_utf8(&data[name_start..name_end]).ok()?;
    Some(BtrfsDirEntry {
        name: String::from(name),
        objectid: dir_item.location.objectid,
        file_type: dir_item.dir_type,
    })
}
