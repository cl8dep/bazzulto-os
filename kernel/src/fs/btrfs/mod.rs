// fs/btrfs/mod.rs — Btrfs filesystem driver for the Bazzulto kernel.
//
// Implements a single-device, uncompressed Btrfs read-write driver that
// integrates with the kernel VFS via the `Inode` trait.
//
// Supported features:
//   - Superblock parsing and validation (CRC32C).
//   - Chunk tree bootstrapping and logical → physical mapping.
//   - B-tree search and iteration.
//   - Read: inode metadata, directory listing, file data (inline + regular extents).
//   - Write: file create, directory create, file write, unlink.
//   - Copy-on-write leaf mutation with superblock commit.
//
// Not supported in v1.0:
//   - Compression (zlib, lzo, zstd).
//   - RAID / multi-device.
//   - Snapshots / subvolumes beyond the default (objectid 5).
//   - Leaf splitting (fails if a leaf is full — relies on mkfs leaving room).
//   - Free space tree (uses simple bump allocator per chunk).
//   - Reflinks / deduplication.
//
// Reference: btrfs on-disk format, Linux kernel fs/btrfs/.

pub mod btree;
pub mod chunk;
pub mod crc32c;
pub mod ondisk;
pub mod read;
pub mod superblock;
pub mod write;

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::inode::{
    alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType,
};
use crate::hal::disk::BlockDevice;
use crate::sync::SpinLock;

use self::chunk::ChunkMap;
use self::ondisk::*;
use self::write::ChunkAllocator;

// ---------------------------------------------------------------------------
// BtrfsVolume — per-mount state
// ---------------------------------------------------------------------------

/// All mutable state for a mounted Btrfs volume.
///
/// Shared across all `BtrfsInode` instances via `Arc<SpinLock<…>>`.
pub struct BtrfsVolume {
    disk: Arc<dyn BlockDevice>,
    partition_start_lba: u64,
    pub superblock: BtrfsSuperblock,
    pub chunk_map: ChunkMap,
    pub generation: u64,
    pub root_tree_root: u64,
    pub root_tree_level: u8,
    pub fs_tree_root: u64,
    pub fs_tree_level: u8,
    pub allocator: ChunkAllocator,
}

impl BtrfsVolume {
    fn nodesize(&self) -> u32 {
        self.superblock.nodesize
    }

    /// Write a B-tree node using the volume's allocator and disk.
    fn write_node(&mut self, node_data: &mut Vec<u8>) -> Option<u64> {
        let nodesize = self.nodesize();
        write::write_node(
            &*self.disk, self.partition_start_lba, &self.chunk_map,
            &mut self.allocator, nodesize, node_data,
        )
    }

    /// Create an extent for file data.
    fn create_extent(&mut self, generation: u64, data: &[u8]) -> Option<Vec<u8>> {
        let nodesize = self.nodesize();
        write::create_extent_for_data(
            &*self.disk, self.partition_start_lba, &self.chunk_map,
            &mut self.allocator, generation, data, nodesize,
        )
    }

    /// Update root tree and commit superblock.
    fn commit(&mut self, new_fs_root: u64, new_fs_level: u8) {
        self.fs_tree_root = new_fs_root;
        let generation = self.generation;
        let nodesize = self.nodesize();
        let bytes_used = self.allocator.bytes_used(
            BTRFS_BLOCK_GROUP_DATA | BTRFS_BLOCK_GROUP_METADATA,
        );

        // Update root tree entry for FS tree.
        if let Some((new_root_tree, new_root_level)) = self.update_root_tree_fs_entry(
            new_fs_root, new_fs_level, generation,
        ) {
            self.root_tree_root = new_root_tree;
            self.root_tree_level = new_root_level;
        }

        let new_gen = generation + 1;
        write::commit_superblock(
            &*self.disk, self.partition_start_lba, &self.superblock,
            self.root_tree_root, self.root_tree_level, new_gen, bytes_used,
        );
        self.generation = new_gen;
    }

    /// Update the root tree's FS tree entry to point to a new root node.
    fn update_root_tree_fs_entry(
        &mut self,
        new_fs_root: u64,
        new_fs_level: u8,
        generation: u64,
    ) -> Option<(u64, u8)> {
        let nodesize = self.nodesize();
        let key = BtrfsKey::new(BTRFS_FS_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0);
        let result = btree::search_first_ge(
            &*self.disk, self.partition_start_lba, &self.chunk_map, nodesize,
            self.root_tree_root, self.root_tree_level, &key,
        )?;

        let (_, data) = btree::get_item_data(&result.leaf, result.slot)?;
        if data.len() < BtrfsRootItem::SIZE { return None; }

        let mut root_item = BtrfsRootItem::from_bytes(data);
        root_item.bytenr = new_fs_root;
        root_item.level = new_fs_level;
        root_item.generation = generation;
        root_item.generation_v2 = generation;
        root_item.ctransid = generation;

        let mut new_data = vec![0u8; BtrfsRootItem::SIZE];
        root_item.to_bytes(&mut new_data);

        let header = BtrfsHeader::from_bytes(&result.leaf);
        let nritems = header.nritems as usize;
        let mut items: Vec<(BtrfsKey, Vec<u8>)> = Vec::with_capacity(nritems);
        for i in 0..nritems {
            if let Some((k, d)) = btree::get_item_data(&result.leaf, i) {
                if i == result.slot {
                    items.push((k, new_data.clone()));
                } else {
                    items.push((k, d.to_vec()));
                }
            }
        }
        let refs: Vec<(BtrfsKey, &[u8])> = items.iter()
            .map(|(k, d)| (*k, d.as_slice())).collect();
        let fsid = self.superblock.fsid;
        let mut new_leaf = write::build_leaf(
            nodesize, header.owner, generation, &fsid, &refs,
        );

        let new_root_tree = self.write_node(&mut new_leaf)?;
        Some((new_root_tree, 0))
    }

    /// Allocate a new objectid for the FS tree.
    fn allocate_objectid(&self) -> u64 {
        BTRFS_FIRST_FREE_OBJECTID + self.generation * 100 + 1
    }

    /// Find the next DIR_INDEX sequence number for a directory.
    fn find_next_dir_index(&self, dir_objectid: u64) -> u64 {
        let nodesize = self.nodesize();
        let search_key = BtrfsKey::new(dir_objectid, BTRFS_DIR_INDEX_KEY, u64::MAX);
        let result = btree::search_slot(
            &*self.disk, self.partition_start_lba, &self.chunk_map,
            nodesize, self.fs_tree_root, self.fs_tree_level, &search_key,
        );
        match result {
            Some(r) if r.slot > 0 => {
                if let Some((key, _)) = btree::get_item_data(&r.leaf, r.slot - 1) {
                    if key.objectid == dir_objectid && key.item_type == BTRFS_DIR_INDEX_KEY {
                        return key.offset + 1;
                    }
                }
                2
            }
            _ => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// BtrfsInode — kernel VFS inode
// ---------------------------------------------------------------------------

/// A VFS inode backed by a Btrfs FS tree object.
pub struct BtrfsInode {
    vfs_inode_number: u64,
    btrfs_objectid: u64,
    volume: Arc<SpinLock<BtrfsVolume>>,
}

unsafe impl Send for BtrfsInode {}
unsafe impl Sync for BtrfsInode {}

impl BtrfsInode {
    fn new(btrfs_objectid: u64, volume: Arc<SpinLock<BtrfsVolume>>) -> Arc<dyn Inode> {
        Arc::new(BtrfsInode {
            vfs_inode_number: alloc_inode_number(),
            btrfs_objectid,
            volume,
        })
    }

    fn make_child(&self, child_objectid: u64) -> Arc<dyn Inode> {
        BtrfsInode::new(child_objectid, Arc::clone(&self.volume))
    }
}

impl Inode for BtrfsInode {
    fn inode_type(&self) -> InodeType {
        let vol = self.volume.lock();
        let inode = read::read_inode(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            vol.nodesize(), vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid,
        );
        match inode {
            Some(i) if i.is_directory() => InodeType::Directory,
            Some(i) if i.is_symlink() => InodeType::Symlink,
            _ => InodeType::RegularFile,
        }
    }

    fn stat(&self) -> InodeStat {
        let vol = self.volume.lock();
        let inode = read::read_inode(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            vol.nodesize(), vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid,
        );
        match inode {
            Some(i) => InodeStat {
                inode_number: self.vfs_inode_number,
                size: i.size,
                mode: i.mode as u64,
                nlinks: i.nlink as u64,
                uid: i.uid,
                gid: i.gid,
            },
            None => InodeStat {
                inode_number: self.vfs_inode_number,
                size: 0,
                mode: 0o100644,
                nlinks: 1,
                uid: 0,
                gid: 0,
            },
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let vol = self.volume.lock();
        let bytes = read::read_file(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            vol.nodesize(), vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid, offset, buf,
        );
        Ok(bytes)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut vol = self.volume.lock();
        let generation = vol.generation;
        let nodesize = vol.nodesize();

        // Read current inode.
        let inode = read::read_inode(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid,
        ).ok_or(FsError::NotFound)?;

        // Create extent data.
        let extent_data = vol.create_extent(generation, buf)
            .ok_or(FsError::OutOfMemory)?;

        let extent_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_EXTENT_DATA_KEY, offset);

        // Search for FS tree leaf.
        let search_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let result = btree::search_slot(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level, &search_key,
        ).ok_or(FsError::IoError)?;

        // Insert extent item into leaf.
        let new_leaf = write::insert_item_into_leaf(
            &result.leaf, nodesize, &extent_key, &extent_data,
        ).ok_or(FsError::OutOfMemory)?;

        // Update inode size.
        let new_size = (offset + buf.len() as u64).max(inode.size);
        let mut updated_leaf = update_inode_size_in_leaf(
            &new_leaf, nodesize, self.btrfs_objectid, new_size, generation,
        ).unwrap_or(new_leaf);

        // Write CoW leaf.
        let new_root = vol.write_node(&mut updated_leaf).ok_or(FsError::IoError)?;

        // Commit.
        vol.commit(new_root, 0);
        Ok(buf.len())
    }

    fn truncate(&self, new_size: u64) -> Result<(), FsError> {
        let mut vol = self.volume.lock();
        let nodesize = vol.nodesize();
        let generation = vol.generation;

        let search_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let result = btree::search_slot(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level, &search_key,
        ).ok_or(FsError::IoError)?;

        let mut updated_leaf = update_inode_size_in_leaf(
            &result.leaf, nodesize, self.btrfs_objectid, new_size, generation,
        ).ok_or(FsError::IoError)?;

        let new_root = vol.write_node(&mut updated_leaf).ok_or(FsError::IoError)?;
        vol.commit(new_root, 0);
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let vol = self.volume.lock();
        let entry = read::lookup_dir(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            vol.nodesize(), vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid, name,
        )?;
        drop(vol);
        Some(self.make_child(entry.objectid))
    }

    fn readdir(&self, index: usize) -> Option<DirEntry> {
        let vol = self.volume.lock();
        let entry = read::readdir_index(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            vol.nodesize(), vol.fs_tree_root, vol.fs_tree_level,
            self.btrfs_objectid, index,
        )?;
        Some(DirEntry {
            name: entry.name,
            inode_type: match entry.file_type {
                BTRFS_FT_DIR => InodeType::Directory,
                BTRFS_FT_SYMLINK => InodeType::Symlink,
                BTRFS_FT_FIFO => InodeType::Fifo,
                BTRFS_FT_CHRDEV => InodeType::CharDevice,
                _ => InodeType::RegularFile,
            },
            inode_number: entry.objectid,
        })
    }

    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let mut vol = self.volume.lock();
        let nodesize = vol.nodesize();
        let generation = vol.generation;
        let new_objectid = vol.allocate_objectid();
        let next_index = vol.find_next_dir_index(self.btrfs_objectid);

        let inode_item = make_inode_item(generation, 0o100644, 0);
        let inode_ref_data = make_inode_ref(name, next_index);
        let dir_item_data = make_dir_item(new_objectid, BTRFS_FT_REG_FILE, generation, name);
        let dir_index_data = dir_item_data.clone();

        let dir_item_key = BtrfsKey::new(
            self.btrfs_objectid, BTRFS_DIR_ITEM_KEY, btrfs_name_hash(name.as_bytes()),
        );
        let dir_index_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_DIR_INDEX_KEY, next_index);
        let inode_item_key = BtrfsKey::new(new_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let inode_ref_key = BtrfsKey::new(new_objectid, BTRFS_INODE_REF_KEY, self.btrfs_objectid);

        let search_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let result = btree::search_slot(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level, &search_key,
        ).ok_or(FsError::IoError)?;

        let mut leaf = result.leaf;
        for (key, data) in &[
            (dir_item_key, dir_item_data.as_slice()),
            (dir_index_key, dir_index_data.as_slice()),
            (inode_item_key, inode_item.as_slice()),
            (inode_ref_key, inode_ref_data.as_slice()),
        ] {
            leaf = write::insert_item_into_leaf(&leaf, nodesize, key, data)
                .ok_or(FsError::OutOfMemory)?;
        }

        let new_root = vol.write_node(&mut leaf).ok_or(FsError::IoError)?;
        vol.commit(new_root, 0);
        drop(vol);
        Ok(self.make_child(new_objectid))
    }

    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let mut vol = self.volume.lock();
        let nodesize = vol.nodesize();
        let generation = vol.generation;
        let new_objectid = vol.allocate_objectid();
        let next_index = vol.find_next_dir_index(self.btrfs_objectid);

        let inode_item = make_inode_item(generation, 0o040755, 0);
        let inode_ref_data = make_inode_ref(name, next_index);
        let dir_item_data = make_dir_item(new_objectid, BTRFS_FT_DIR, generation, name);
        let dir_index_data = dir_item_data.clone();

        let dir_item_key = BtrfsKey::new(
            self.btrfs_objectid, BTRFS_DIR_ITEM_KEY, btrfs_name_hash(name.as_bytes()),
        );
        let dir_index_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_DIR_INDEX_KEY, next_index);
        let inode_item_key = BtrfsKey::new(new_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let inode_ref_key = BtrfsKey::new(new_objectid, BTRFS_INODE_REF_KEY, self.btrfs_objectid);

        let search_key = BtrfsKey::new(self.btrfs_objectid, BTRFS_INODE_ITEM_KEY, 0);
        let result = btree::search_slot(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level, &search_key,
        ).ok_or(FsError::IoError)?;

        let mut leaf = result.leaf;
        for (key, data) in &[
            (dir_item_key, dir_item_data.as_slice()),
            (dir_index_key, dir_index_data.as_slice()),
            (inode_item_key, inode_item.as_slice()),
            (inode_ref_key, inode_ref_data.as_slice()),
        ] {
            leaf = write::insert_item_into_leaf(&leaf, nodesize, key, data)
                .ok_or(FsError::OutOfMemory)?;
        }

        let new_root = vol.write_node(&mut leaf).ok_or(FsError::IoError)?;
        vol.commit(new_root, 0);
        drop(vol);
        Ok(self.make_child(new_objectid))
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let mut vol = self.volume.lock();
        let nodesize = vol.nodesize();

        let dir_item_key = BtrfsKey::new(
            self.btrfs_objectid, BTRFS_DIR_ITEM_KEY, btrfs_name_hash(name.as_bytes()),
        );
        let result = btree::search_exact(
            &*vol.disk, vol.partition_start_lba, &vol.chunk_map,
            nodesize, vol.fs_tree_root, vol.fs_tree_level, &dir_item_key,
        ).ok_or(FsError::NotFound)?;

        let mut leaf = write::delete_item_from_leaf(&result.leaf, nodesize, result.slot);
        let new_root = vol.write_node(&mut leaf).ok_or(FsError::IoError)?;
        vol.commit(new_root, 0);
        Ok(())
    }

    fn fsync(&self) -> Result<(), FsError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public API: probe and mount
// ---------------------------------------------------------------------------

/// Quick check for Btrfs magic at the given partition.
pub fn btrfs_probe(disk: &Arc<dyn BlockDevice>, partition_start_lba: u64) -> bool {
    superblock::probe_btrfs_magic(&**disk, partition_start_lba)
}

/// Mount a Btrfs filesystem and return the root directory inode.
pub fn btrfs_mount(
    disk: Arc<dyn BlockDevice>,
    partition_start_lba: u64,
) -> Option<Arc<dyn Inode>> {
    // Read and validate superblock.
    let sb = superblock::read_superblock(&*disk, partition_start_lba).ok()?;
    let nodesize = sb.nodesize;

    // Bootstrap chunk map from sys_chunk_array.
    let mut chunk_map = chunk::parse_sys_chunk_array(
        &sb.sys_chunk_array, sb.sys_chunk_array_size,
    );

    // Load full chunk tree.  We need a temporary clone of the bootstrap map
    // for the read closure because load_chunk_tree mutates chunk_map.
    let chunk_root = sb.chunk_root;
    let chunk_root_level = sb.chunk_root_level;
    let disk_ref = Arc::clone(&disk);
    {
        // Build a snapshot of current mappings for the read closure.
        let bootstrap_map = chunk_map.clone();
        chunk::load_chunk_tree(
            &mut chunk_map, chunk_root, chunk_root_level, nodesize,
            &|logical| {
                let physical = bootstrap_map.logical_to_physical(logical)?;
                let sector = partition_start_lba + physical / 512;
                let sectors = (nodesize as u64 + 511) / 512;
                let mut buf = vec![0u8; nodesize as usize];
                if !disk_ref.read_sectors(sector, sectors as u32, &mut buf) {
                    return None;
                }
                Some(buf)
            },
        );
    }

    // Find FS tree root from root tree.
    let root_tree_root = sb.root;
    let root_tree_level = sb.root_level;
    let fs_tree_key = BtrfsKey::new(BTRFS_FS_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0);
    let fs_root_item = btree::search_first_ge(
        &*disk, partition_start_lba, &chunk_map, nodesize,
        root_tree_root, root_tree_level, &fs_tree_key,
    )?;

    let (_k, root_item_data) = btree::get_item_data(&fs_root_item.leaf, fs_root_item.slot)?;
    if root_item_data.len() < BtrfsRootItem::SIZE { return None; }
    let root_item = BtrfsRootItem::from_bytes(root_item_data);

    // Build allocator with initially-used ranges.
    let used_ranges = vec![
        (sb.chunk_root, nodesize as u64),
        (sb.root, nodesize as u64),
        (root_item.bytenr, nodesize as u64),
    ];
    let allocator = ChunkAllocator::new(&chunk_map, &used_ranges);

    let volume = BtrfsVolume {
        disk,
        partition_start_lba,
        superblock: sb,
        chunk_map,
        generation: root_item.generation,
        root_tree_root,
        root_tree_level,
        fs_tree_root: root_item.bytenr,
        fs_tree_level: root_item.level,
        allocator,
    };

    let shared = Arc::new(SpinLock::new(volume));
    Some(BtrfsInode::new(BTRFS_FIRST_FREE_OBJECTID, shared))
}

/// Return the filesystem label.
pub fn btrfs_label(disk: &dyn BlockDevice, partition_start_lba: u64) -> Option<String> {
    let sb = superblock::read_superblock(disk, partition_start_lba).ok()?;
    let end = sb.label.iter().position(|&b| b == 0).unwrap_or(BTRFS_LABEL_SIZE);
    let label = core::str::from_utf8(&sb.label[..end]).ok()?;
    Some(String::from(label.trim()))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn make_inode_item(generation: u64, mode: u32, size: u64) -> Vec<u8> {
    let inode = BtrfsInodeItem {
        generation, transid: generation, size, nbytes: size,
        block_group: 0, nlink: 1, uid: 0, gid: 0, mode, rdev: 0,
        flags: 0, sequence: 0, reserved: [0; 4],
        atime: BtrfsTimespec::default(), ctime: BtrfsTimespec::default(),
        mtime: BtrfsTimespec::default(), otime: BtrfsTimespec::default(),
    };
    let mut buf = vec![0u8; BtrfsInodeItem::SIZE];
    inode.to_bytes(&mut buf);
    buf
}

fn make_inode_ref(name: &str, index: u64) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut buf = vec![0u8; BtrfsInodeRef::HEADER_SIZE + name_bytes.len()];
    buf[0..8].copy_from_slice(&index.to_le_bytes());
    buf[8..10].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    buf[BtrfsInodeRef::HEADER_SIZE..].copy_from_slice(name_bytes);
    buf
}

fn make_dir_item(child_objectid: u64, file_type: u8, generation: u64, name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut buf = vec![0u8; BtrfsDirItem::HEADER_SIZE + name_bytes.len()];
    let location = BtrfsKey::new(child_objectid, BTRFS_INODE_ITEM_KEY, 0);
    location.to_bytes(&mut buf[0..17]);
    buf[17..25].copy_from_slice(&generation.to_le_bytes());
    buf[25..27].copy_from_slice(&0u16.to_le_bytes());
    buf[27..29].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    buf[29] = file_type;
    buf[BtrfsDirItem::HEADER_SIZE..].copy_from_slice(name_bytes);
    buf
}

fn update_inode_size_in_leaf(
    leaf: &[u8], nodesize: u32, objectid: u64, new_size: u64, generation: u64,
) -> Option<Vec<u8>> {
    let header = BtrfsHeader::from_bytes(leaf);
    let nritems = header.nritems as usize;
    for i in 0..nritems {
        let (key, data) = btree::get_item_data(leaf, i)?;
        if key.objectid == objectid && key.item_type == BTRFS_INODE_ITEM_KEY {
            let mut inode = BtrfsInodeItem::from_bytes(data);
            inode.size = new_size;
            inode.nbytes = new_size;
            inode.transid = generation;
            let mut new_data = vec![0u8; BtrfsInodeItem::SIZE];
            inode.to_bytes(&mut new_data);

            let mut items: Vec<(BtrfsKey, Vec<u8>)> = Vec::with_capacity(nritems);
            for j in 0..nritems {
                if let Some((k, d)) = btree::get_item_data(leaf, j) {
                    if j == i { items.push((k, new_data.clone())); }
                    else { items.push((k, d.to_vec())); }
                }
            }
            let refs: Vec<(BtrfsKey, &[u8])> = items.iter()
                .map(|(k, d)| (*k, d.as_slice())).collect();
            return Some(write::build_leaf(
                nodesize, header.owner, generation, &header.fsid, &refs,
            ));
        }
    }
    None
}
