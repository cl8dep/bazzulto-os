//! mkfs_btrfs — Create a Btrfs disk image populated with host files.
//!
//! Usage: mkfs_btrfs <output.img> <size_mb> [--label LABEL] [host:target ...]
//!                                           [DIR:/path] [TREE:host:target]
//!
//! Creates a raw disk image formatted as a minimal Btrfs filesystem.
//! Single-device, uncompressed, nodesize=16384, sectorsize=4096.
//!
//! The image is compatible with the Bazzulto kernel's btrfs driver.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// On-disk constants
// ---------------------------------------------------------------------------

const BTRFS_MAGIC: u64 = 0x4D5F53665248425F; // "_BHRfS_M"
const BTRFS_SUPER_INFO_OFFSET: u64 = 0x10000;
const BTRFS_SUPER_INFO_SIZE: usize = 4096;
const BTRFS_CSUM_SIZE: usize = 32;
const BTRFS_UUID_SIZE: usize = 16;
const BTRFS_LABEL_SIZE: usize = 256;
const BTRFS_SYSTEM_CHUNK_ARRAY_SIZE: usize = 2048;

const NODESIZE: u32 = 16384;
const SECTORSIZE: u32 = 4096;
const STRIPESIZE: u32 = 65536;

// Object IDs.
const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;
const BTRFS_EXTENT_TREE_OBJECTID: u64 = 2;
const BTRFS_CHUNK_TREE_OBJECTID: u64 = 3;
const BTRFS_FS_TREE_OBJECTID: u64 = 5;
const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;
const BTRFS_FIRST_CHUNK_TREE_OBJECTID: u64 = 256;
const BTRFS_DEV_ITEMS_OBJECTID: u64 = 1;

// Item types.
const BTRFS_INODE_ITEM_KEY: u8 = 1;
const BTRFS_INODE_REF_KEY: u8 = 12;
const BTRFS_DIR_ITEM_KEY: u8 = 84;
const BTRFS_DIR_INDEX_KEY: u8 = 96;
const BTRFS_EXTENT_DATA_KEY: u8 = 108;
const BTRFS_ROOT_ITEM_KEY: u8 = 132;
const BTRFS_DEV_ITEM_KEY: u8 = 216;
const BTRFS_CHUNK_ITEM_KEY: u8 = 228;

// Block group flags.
const BTRFS_BLOCK_GROUP_DATA: u64 = 1 << 0;
const BTRFS_BLOCK_GROUP_SYSTEM: u64 = 1 << 1;
const BTRFS_BLOCK_GROUP_METADATA: u64 = 1 << 2;

// File extent types.
const BTRFS_FILE_EXTENT_INLINE: u8 = 0;
const BTRFS_FILE_EXTENT_REG: u8 = 1;

// Dir types.
const BTRFS_FT_REG_FILE: u8 = 1;
const BTRFS_FT_DIR: u8 = 2;

// Incompat flags.
const BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;
const BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;
const BTRFS_FEATURE_INCOMPAT_NO_HOLES: u64 = 1 << 9;

// Header/item sizes.
const HEADER_SIZE: usize = 101;
const ITEM_SIZE: usize = 25;
const KEY_SIZE: usize = 17;
const KEYPTR_SIZE: usize = 33;
const INODE_ITEM_SIZE: usize = 160;
const ROOT_ITEM_SIZE: usize = 439;
const DEV_ITEM_SIZE: usize = 98;
const CHUNK_ITEM_SIZE: usize = 48;
const STRIPE_SIZE: usize = 32;
const DIR_ITEM_HEADER_SIZE: usize = 30;
const INODE_REF_HEADER_SIZE: usize = 10;

// Superblock field offsets.
const SB_OFF_CSUM: usize = 0x00;
const SB_OFF_FSID: usize = 0x20;
const SB_OFF_BYTENR: usize = 0x30;
const SB_OFF_FLAGS: usize = 0x38;
const SB_OFF_MAGIC: usize = 0x40;
const SB_OFF_GENERATION: usize = 0x48;
const SB_OFF_ROOT: usize = 0x50;
const SB_OFF_CHUNK_ROOT: usize = 0x58;
const SB_OFF_LOG_ROOT: usize = 0x60;
const SB_OFF_TOTAL_BYTES: usize = 0x70;
const SB_OFF_BYTES_USED: usize = 0x78;
const SB_OFF_ROOT_DIR_OBJECTID: usize = 0x80;
const SB_OFF_NUM_DEVICES: usize = 0x88;
const SB_OFF_SECTORSIZE: usize = 0x90;
const SB_OFF_NODESIZE: usize = 0x94;
const SB_OFF_LEAFSIZE: usize = 0x98;
const SB_OFF_STRIPESIZE: usize = 0x9C;
const SB_OFF_SYS_CHUNK_ARRAY_SIZE: usize = 0xA0;
const SB_OFF_CHUNK_ROOT_GENERATION: usize = 0xA4;
const SB_OFF_COMPAT_FLAGS: usize = 0xAC;
const SB_OFF_COMPAT_RO_FLAGS: usize = 0xB4;
const SB_OFF_INCOMPAT_FLAGS: usize = 0xBC;
const SB_OFF_CSUM_TYPE: usize = 0xC4;
const SB_OFF_ROOT_LEVEL: usize = 0xC6;
const SB_OFF_CHUNK_ROOT_LEVEL: usize = 0xC7;
const SB_OFF_LOG_ROOT_LEVEL: usize = 0xC8;
const SB_OFF_DEV_ITEM: usize = 0xC9;
const SB_OFF_LABEL: usize = 0x12B;
const SB_OFF_CACHE_GENERATION: usize = 0x22B;
const SB_OFF_UUID_TREE_GENERATION: usize = 0x233;
const SB_OFF_METADATA_UUID: usize = 0x23B;
const SB_OFF_SYS_CHUNK_ARRAY: usize = 0x2FB;

// ---------------------------------------------------------------------------
// Chunk layout for a fresh filesystem.
// Logical addresses == physical addresses for single-device.
// ---------------------------------------------------------------------------

/// System chunk: 1 MiB offset, 8 MiB length.
const SYSTEM_CHUNK_LOGICAL: u64 = 0x100000;
const SYSTEM_CHUNK_LENGTH: u64 = 8 * 1024 * 1024;

/// Metadata chunk: 16 MiB offset, 256 MiB length.
const META_CHUNK_LOGICAL: u64 = 0x1000000;
const META_CHUNK_LENGTH: u64 = 256 * 1024 * 1024;

/// Data chunk: 272 MiB offset, rest of disk.
const DATA_CHUNK_LOGICAL: u64 = 0x11000000;

// Tree node positions within their chunks.
const CHUNK_TREE_LEAF: u64 = SYSTEM_CHUNK_LOGICAL;
const ROOT_TREE_LEAF: u64 = META_CHUNK_LOGICAL;
const FS_TREE_LEAF: u64 = META_CHUNK_LOGICAL + NODESIZE as u64;
const EXTENT_TREE_LEAF: u64 = META_CHUNK_LOGICAL + 2 * NODESIZE as u64;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: mkfs_btrfs <output.img> <size_mb> [--label LABEL] [host:target ...] [DIR:path] [TREE:host:target]");
        std::process::exit(1);
    }

    let output_path = &args[1];
    let size_mb: u64 = args[2].parse().expect("size_mb must be a number");
    let total_bytes = size_mb * 1024 * 1024;

    let mut label = [0u8; BTRFS_LABEL_SIZE];
    let default_label = b"BAZZULTO";
    label[..default_label.len()].copy_from_slice(default_label);

    let mut mappings: Vec<(String, String)> = Vec::new();
    let mut dir_only: Vec<String> = Vec::new();
    let mut tree_mappings: Vec<(String, String)> = Vec::new();

    let mut iter = args[3..].iter();
    while let Some(arg) = iter.next() {
        if arg == "--label" {
            if let Some(l) = iter.next() {
                label = [0u8; BTRFS_LABEL_SIZE];
                let bytes = l.as_bytes();
                let n = bytes.len().min(BTRFS_LABEL_SIZE - 1);
                label[..n].copy_from_slice(&bytes[..n]);
            }
        } else if let Some(target) = arg.strip_prefix("DIR:") {
            dir_only.push(target.to_string());
        } else if let Some(rest) = arg.strip_prefix("TREE:") {
            if let Some(pos) = rest.find(':') {
                let host = rest[..pos].to_string();
                let target = rest[pos+1..].to_string();
                tree_mappings.push((host, target));
            }
        } else if let Some(pos) = arg.find(':') {
            let host = arg[..pos].to_string();
            let target = arg[pos+1..].to_string();
            mappings.push((host, target));
        }
    }

    // Generate UUIDs.
    let fsid = uuid::Uuid::new_v4().into_bytes();
    let dev_uuid = uuid::Uuid::new_v4().into_bytes();
    let chunk_tree_uuid = uuid::Uuid::new_v4().into_bytes();

    let generation: u64 = 1;
    let data_chunk_length = total_bytes.saturating_sub(DATA_CHUNK_LOGICAL);

    // ---------------------------------------------------------------------------
    // Build virtual filesystem tree.
    // ---------------------------------------------------------------------------
    let mut vfs = VirtualFs::new();

    for path in &dir_only {
        vfs.ensure_directory(path);
    }
    for (host, target) in &mappings {
        let data = fs::read(host).unwrap_or_else(|e| {
            eprintln!("mkfs_btrfs: cannot read '{}': {}", host, e);
            std::process::exit(1);
        });
        vfs.add_file(target, data);
    }
    for (host_dir, target_prefix) in &tree_mappings {
        add_tree_recursive(&mut vfs, Path::new(host_dir), target_prefix);
    }

    // ---------------------------------------------------------------------------
    // Build FS tree items.
    // ---------------------------------------------------------------------------
    let mut fs_items: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut data_offset: u64 = DATA_CHUNK_LOGICAL; // bump allocator for data extents
    let mut next_dir_index: BTreeMap<u64, u64> = BTreeMap::new();

    // Root directory inode (objectid 256).
    let root_dir_objectid = BTRFS_FIRST_FREE_OBJECTID;
    fs_items.push((
        Key::new(root_dir_objectid, BTRFS_INODE_ITEM_KEY, 0),
        make_inode_item(generation, 0o040755, 0),
    ));
    // INODE_REF for root dir → itself (parent is root_dir_objectid).
    fs_items.push((
        Key::new(root_dir_objectid, BTRFS_INODE_REF_KEY, root_dir_objectid),
        make_inode_ref("..", 0),
    ));

    // Assign objectids and build items for all VFS entries.
    let mut objectid_counter = root_dir_objectid + 1;
    let mut path_to_objectid: BTreeMap<String, u64> = BTreeMap::new();
    path_to_objectid.insert("/".to_string(), root_dir_objectid);

    // Create entries sorted by path depth.
    let mut sorted_paths: Vec<String> = vfs.entries.keys().cloned().collect();
    sorted_paths.sort_by(|a, b| {
        let da = a.matches('/').count();
        let db = b.matches('/').count();
        da.cmp(&db).then(a.cmp(b))
    });

    for path in &sorted_paths {
        let entry = &vfs.entries[path];
        let objectid = objectid_counter;
        objectid_counter += 1;
        path_to_objectid.insert(path.clone(), objectid);

        let parent_path = parent_of(path);
        let parent_oid = *path_to_objectid.get(&parent_path).unwrap_or(&root_dir_objectid);
        let name = filename_of(path);

        let (mode, file_type) = if entry.is_dir {
            (0o040755u32, BTRFS_FT_DIR)
        } else {
            // File permissions based on path:
            //   /system/bin/su → 4755 (setuid root — privilege escalation)
            //   /system/bin/*  → 0755 (executables)
            //   */shadow       → 0600 (passwords, root only)
            //   everything else → 0644 (world-readable)
            let file_mode = if path == "/system/bin/su" {
                0o104755u32  // Setuid root: S_ISUID + rwxr-xr-x
            } else if path.contains("/system/bin/") || path.ends_with(".sh") {
                0o100755u32  // Executables need +x for DAC
            } else if path.contains("/home/user/") && !entry.is_dir {
                0o100755u32  // User binaries need +x (BPM test programs)
            } else if path.ends_with("/shadow") || path.ends_with("/root_secret.txt") {
                0o100600u32  // Root only — tests DAC denial for uid=1000
            } else {
                0o100644u32
            };
            (file_mode, BTRFS_FT_REG_FILE)
        };

        // INODE_ITEM.
        fs_items.push((
            Key::new(objectid, BTRFS_INODE_ITEM_KEY, 0),
            make_inode_item(generation, mode, entry.data.len() as u64),
        ));

        // INODE_REF.
        let dir_idx = next_dir_index.entry(parent_oid).or_insert(2);
        let this_index = *dir_idx;
        *dir_idx += 1;

        fs_items.push((
            Key::new(objectid, BTRFS_INODE_REF_KEY, parent_oid),
            make_inode_ref(name, this_index),
        ));

        // DIR_ITEM in parent.
        let name_hash = btrfs_name_hash(name.as_bytes());
        fs_items.push((
            Key::new(parent_oid, BTRFS_DIR_ITEM_KEY, name_hash),
            make_dir_item(objectid, file_type, generation, name),
        ));

        // DIR_INDEX in parent.
        fs_items.push((
            Key::new(parent_oid, BTRFS_DIR_INDEX_KEY, this_index),
            make_dir_item(objectid, file_type, generation, name),
        ));

        // EXTENT_DATA for files.
        if !entry.is_dir && !entry.data.is_empty() {
            let inline_max = (NODESIZE as usize / 4).min(2048);
            if entry.data.len() <= inline_max {
                fs_items.push((
                    Key::new(objectid, BTRFS_EXTENT_DATA_KEY, 0),
                    make_inline_extent(generation, &entry.data),
                ));
            } else {
                let aligned = ((entry.data.len() as u64) + 4095) & !4095;
                fs_items.push((
                    Key::new(objectid, BTRFS_EXTENT_DATA_KEY, 0),
                    make_regular_extent(generation, data_offset, aligned, entry.data.len() as u64),
                ));
                // Remember to write data later.
                data_offset += aligned;
            }
        }
    }

    // Sort FS tree items by key.
    fs_items.sort_by(|a, b| a.0.cmp(&b.0));

    // ---------------------------------------------------------------------------
    // Build B-tree leaves.
    // ---------------------------------------------------------------------------
    // Build FS tree — may span multiple leaves for large filesystems.
    let fs_leaves = split_into_leaves(NODESIZE, BTRFS_FS_TREE_OBJECTID, generation, &fsid, &fs_items);
    let fs_tree_level: u8;
    let fs_tree_root_addr: u64;
    let mut fs_tree_nodes: Vec<(u64, Vec<u8>)> = Vec::new(); // (logical_addr, node_data)

    // Allocate addresses for leaves starting at FS_TREE_LEAF.
    let mut next_meta_addr = FS_TREE_LEAF;
    for (leaf, _first_key) in &fs_leaves {
        let addr = next_meta_addr;
        let mut node = leaf.clone();
        finalize_node(&mut node, addr);
        fs_tree_nodes.push((addr, node));
        next_meta_addr += NODESIZE as u64;
    }

    if fs_leaves.len() == 1 {
        fs_tree_level = 0;
        fs_tree_root_addr = FS_TREE_LEAF;
    } else {
        // Build internal node pointing to all leaves.
        let children: Vec<(Key, u64)> = fs_leaves.iter()
            .enumerate()
            .map(|(i, (_, first_key))| (*first_key, FS_TREE_LEAF + i as u64 * NODESIZE as u64))
            .collect();
        let mut internal = build_internal_node(
            NODESIZE, BTRFS_FS_TREE_OBJECTID, generation, &fsid, 1, &children,
        );
        let internal_addr = next_meta_addr;
        finalize_node(&mut internal, internal_addr);
        fs_tree_nodes.push((internal_addr, internal));
        next_meta_addr += NODESIZE as u64;
        fs_tree_level = 1;
        fs_tree_root_addr = internal_addr;
    }

    // Extent tree leaf — placed after FS tree nodes.
    let extent_tree_addr = next_meta_addr;
    let mut extent_leaf = build_leaf(NODESIZE, BTRFS_EXTENT_TREE_OBJECTID, generation, &fsid, &[]);
    finalize_node(&mut extent_leaf, extent_tree_addr);
    next_meta_addr += NODESIZE as u64;

    // Root tree items.
    let mut root_items: Vec<(Key, Vec<u8>)> = Vec::new();
    root_items.push((
        Key::new(BTRFS_EXTENT_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
        make_root_item(generation, extent_tree_addr, 0, root_dir_objectid),
    ));
    root_items.push((
        Key::new(BTRFS_FS_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
        make_root_item(generation, fs_tree_root_addr, fs_tree_level, root_dir_objectid),
    ));
    root_items.sort_by(|a, b| a.0.cmp(&b.0));

    let root_tree_addr = next_meta_addr;
    let mut root_leaf = build_leaf(NODESIZE, BTRFS_ROOT_TREE_OBJECTID, generation, &fsid, &root_items);
    finalize_node(&mut root_leaf, root_tree_addr);

    // Chunk tree items.
    let mut chunk_items: Vec<(Key, Vec<u8>)> = Vec::new();
    // System chunk.
    chunk_items.push((
        Key::new(BTRFS_FIRST_CHUNK_TREE_OBJECTID, BTRFS_CHUNK_ITEM_KEY, SYSTEM_CHUNK_LOGICAL),
        make_chunk_item(SYSTEM_CHUNK_LENGTH, BTRFS_BLOCK_GROUP_SYSTEM, SYSTEM_CHUNK_LOGICAL, &dev_uuid),
    ));
    // Metadata chunk.
    chunk_items.push((
        Key::new(BTRFS_FIRST_CHUNK_TREE_OBJECTID, BTRFS_CHUNK_ITEM_KEY, META_CHUNK_LOGICAL),
        make_chunk_item(META_CHUNK_LENGTH, BTRFS_BLOCK_GROUP_METADATA, META_CHUNK_LOGICAL, &dev_uuid),
    ));
    // Data chunk.
    chunk_items.push((
        Key::new(BTRFS_FIRST_CHUNK_TREE_OBJECTID, BTRFS_CHUNK_ITEM_KEY, DATA_CHUNK_LOGICAL),
        make_chunk_item(data_chunk_length, BTRFS_BLOCK_GROUP_DATA, DATA_CHUNK_LOGICAL, &dev_uuid),
    ));
    // Dev item.
    chunk_items.push((
        Key::new(BTRFS_DEV_ITEMS_OBJECTID, BTRFS_DEV_ITEM_KEY, 1),
        make_dev_item_data(1, total_bytes, 0, &dev_uuid, &fsid),
    ));
    chunk_items.sort_by(|a, b| a.0.cmp(&b.0));

    let mut chunk_leaf = build_leaf(NODESIZE, BTRFS_CHUNK_TREE_OBJECTID, generation, &fsid, &chunk_items);
    finalize_node(&mut chunk_leaf, CHUNK_TREE_LEAF);

    // ---------------------------------------------------------------------------
    // Build sys_chunk_array (bootstrap).
    // ---------------------------------------------------------------------------
    let mut sys_chunk_array = [0u8; BTRFS_SYSTEM_CHUNK_ARRAY_SIZE];
    let mut sca_offset = 0;
    // System chunk entry.
    let sca_key = Key::new(BTRFS_FIRST_CHUNK_TREE_OBJECTID, BTRFS_CHUNK_ITEM_KEY, SYSTEM_CHUNK_LOGICAL);
    write_key(&mut sys_chunk_array[sca_offset..], &sca_key);
    sca_offset += KEY_SIZE;
    write_chunk_item_raw(&mut sys_chunk_array[sca_offset..], SYSTEM_CHUNK_LENGTH, BTRFS_BLOCK_GROUP_SYSTEM);
    sca_offset += CHUNK_ITEM_SIZE;
    write_stripe_raw(&mut sys_chunk_array[sca_offset..], 1, SYSTEM_CHUNK_LOGICAL, &dev_uuid);
    sca_offset += STRIPE_SIZE;
    let sys_chunk_array_size = sca_offset as u32;

    // ---------------------------------------------------------------------------
    // Build superblock.
    // ---------------------------------------------------------------------------
    let mut sb = vec![0u8; BTRFS_SUPER_INFO_SIZE];

    write_le64(&mut sb, SB_OFF_BYTENR, BTRFS_SUPER_INFO_OFFSET);
    write_le64(&mut sb, SB_OFF_FLAGS, 0);
    write_le64(&mut sb, SB_OFF_MAGIC, BTRFS_MAGIC);
    write_le64(&mut sb, SB_OFF_GENERATION, generation);
    write_le64(&mut sb, SB_OFF_ROOT, root_tree_addr);
    write_le64(&mut sb, SB_OFF_CHUNK_ROOT, CHUNK_TREE_LEAF);
    write_le64(&mut sb, SB_OFF_LOG_ROOT, 0);
    write_le64(&mut sb, SB_OFF_TOTAL_BYTES, total_bytes);
    let total_nodes = fs_tree_nodes.len() as u64 + 3; // fs tree + extent + root + chunk
    write_le64(&mut sb, SB_OFF_BYTES_USED, total_nodes * NODESIZE as u64);
    write_le64(&mut sb, SB_OFF_ROOT_DIR_OBJECTID, 6);
    write_le64(&mut sb, SB_OFF_NUM_DEVICES, 1);
    write_le32(&mut sb, SB_OFF_SECTORSIZE, SECTORSIZE);
    write_le32(&mut sb, SB_OFF_NODESIZE, NODESIZE);
    write_le32(&mut sb, SB_OFF_LEAFSIZE, NODESIZE);
    write_le32(&mut sb, SB_OFF_STRIPESIZE, STRIPESIZE);
    write_le32(&mut sb, SB_OFF_SYS_CHUNK_ARRAY_SIZE, sys_chunk_array_size);
    write_le64(&mut sb, SB_OFF_CHUNK_ROOT_GENERATION, generation);
    write_le64(&mut sb, SB_OFF_COMPAT_FLAGS, 0);
    write_le64(&mut sb, SB_OFF_COMPAT_RO_FLAGS, 0);
    write_le64(&mut sb, SB_OFF_INCOMPAT_FLAGS,
        BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF |
        BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA |
        BTRFS_FEATURE_INCOMPAT_NO_HOLES);
    write_le16(&mut sb, SB_OFF_CSUM_TYPE, 0); // CRC32C
    sb[SB_OFF_ROOT_LEVEL] = 0; // root tree is always level 0 (one leaf)
    sb[SB_OFF_CHUNK_ROOT_LEVEL] = 0;
    sb[SB_OFF_LOG_ROOT_LEVEL] = 0;

    // FSID.
    sb[SB_OFF_FSID..SB_OFF_FSID + 16].copy_from_slice(&fsid);
    // Metadata UUID = fsid.
    sb[SB_OFF_METADATA_UUID..SB_OFF_METADATA_UUID + 16].copy_from_slice(&fsid);
    // Label.
    sb[SB_OFF_LABEL..SB_OFF_LABEL + BTRFS_LABEL_SIZE].copy_from_slice(&label);
    // Cache generation (u64::MAX means no space cache).
    write_le64(&mut sb, SB_OFF_CACHE_GENERATION, u64::MAX);
    write_le64(&mut sb, SB_OFF_UUID_TREE_GENERATION, 0);

    // Dev item in superblock.
    let dev_item_bytes = make_dev_item_data(1, total_bytes, 0, &dev_uuid, &fsid);
    sb[SB_OFF_DEV_ITEM..SB_OFF_DEV_ITEM + DEV_ITEM_SIZE]
        .copy_from_slice(&dev_item_bytes[..DEV_ITEM_SIZE]);

    // Sys chunk array.
    sb[SB_OFF_SYS_CHUNK_ARRAY..SB_OFF_SYS_CHUNK_ARRAY + sca_offset]
        .copy_from_slice(&sys_chunk_array[..sca_offset]);

    // Compute superblock checksum.
    let csum = btrfs_crc32c_final(&sb[0x20..BTRFS_SUPER_INFO_SIZE]);
    sb[0..4].copy_from_slice(&csum.to_le_bytes());

    // ---------------------------------------------------------------------------
    // Write disk image.
    // ---------------------------------------------------------------------------
    eprintln!("mkfs_btrfs: creating {} ({} MiB, Btrfs)", output_path, size_mb);

    let mut file = OpenOptions::new()
        .create(true).write(true).truncate(true)
        .open(output_path)
        .expect("cannot create output file");

    // Extend to full size.
    file.set_len(total_bytes).expect("cannot set file size");

    // Write superblock.
    file.seek(SeekFrom::Start(BTRFS_SUPER_INFO_OFFSET)).unwrap();
    file.write_all(&sb).unwrap();

    // Write chunk tree leaf.
    file.seek(SeekFrom::Start(CHUNK_TREE_LEAF)).unwrap();
    file.write_all(&chunk_leaf).unwrap();

    // Write FS tree nodes (leaves + optional internal node).
    for (addr, node) in &fs_tree_nodes {
        file.seek(SeekFrom::Start(*addr)).unwrap();
        file.write_all(node).unwrap();
    }

    // Write extent tree leaf.
    file.seek(SeekFrom::Start(extent_tree_addr)).unwrap();
    file.write_all(&extent_leaf).unwrap();

    // Write root tree leaf.
    file.seek(SeekFrom::Start(root_tree_addr)).unwrap();
    file.write_all(&root_leaf).unwrap();

    // Write file data extents.
    let mut data_write_offset = DATA_CHUNK_LOGICAL;
    for path in &sorted_paths {
        let entry = &vfs.entries[path];
        if entry.is_dir || entry.data.is_empty() { continue; }
        let inline_max = (NODESIZE as usize / 4).min(2048);
        if entry.data.len() <= inline_max { continue; }

        file.seek(SeekFrom::Start(data_write_offset)).unwrap();
        file.write_all(&entry.data).unwrap();
        // Pad to 4096.
        let padding = ((entry.data.len() + 4095) & !4095) - entry.data.len();
        if padding > 0 {
            file.write_all(&vec![0u8; padding]).unwrap();
        }
        data_write_offset += ((entry.data.len() as u64) + 4095) & !4095;
    }

    eprintln!("mkfs_btrfs: done — {} files, {} directories",
        vfs.entries.values().filter(|e| !e.is_dir).count(),
        vfs.entries.values().filter(|e| e.is_dir).count());
}

// ---------------------------------------------------------------------------
// Virtual filesystem for collecting entries before writing.
// ---------------------------------------------------------------------------

struct VfsEntry {
    is_dir: bool,
    data: Vec<u8>,
}

struct VirtualFs {
    entries: BTreeMap<String, VfsEntry>,
}

impl VirtualFs {
    fn new() -> Self {
        VirtualFs { entries: BTreeMap::new() }
    }

    fn ensure_directory(&mut self, path: &str) {
        let path = normalize_path(path);
        // Ensure all parent directories exist.
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let mut current = String::new();
        for part in parts {
            current = format!("{}/{}", current, part);
            self.entries.entry(current.clone()).or_insert(VfsEntry {
                is_dir: true,
                data: Vec::new(),
            });
        }
    }

    fn add_file(&mut self, path: &str, data: Vec<u8>) {
        let path = normalize_path(path);
        // Ensure parent directories exist.
        let parent = parent_of(&path);
        if parent != "/" {
            self.ensure_directory(&parent);
        }
        self.entries.insert(path, VfsEntry { is_dir: false, data });
    }
}

fn normalize_path(path: &str) -> String {
    let path = if path.starts_with('/') { path.to_string() } else { format!("/{}", path) };
    // Remove trailing slash.
    if path.len() > 1 && path.ends_with('/') {
        path[..path.len()-1].to_string()
    } else {
        path
    }
}

fn parent_of(path: &str) -> String {
    if let Some(pos) = path.rfind('/') {
        if pos == 0 { "/".to_string() } else { path[..pos].to_string() }
    } else {
        "/".to_string()
    }
}

fn filename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn add_tree_recursive(vfs: &mut VirtualFs, host_dir: &Path, target_prefix: &str) {
    let entries = match fs::read_dir(host_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("mkfs_btrfs: cannot read directory '{}': {}", host_dir.display(), e);
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_type = entry.file_type().unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        let target_path = format!("{}/{}", target_prefix.trim_end_matches('/'), name);

        if file_type.is_dir() {
            vfs.ensure_directory(&target_path);
            add_tree_recursive(vfs, &entry.path(), &target_path);
        } else if file_type.is_file() {
            let data = fs::read(entry.path()).unwrap_or_else(|e| {
                eprintln!("mkfs_btrfs: cannot read '{}': {}", entry.path().display(), e);
                Vec::new()
            });
            vfs.add_file(&target_path, data);
        }
    }
}

// ---------------------------------------------------------------------------
// Key type (host-side, for sorting).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    objectid: u64,
    item_type: u8,
    offset: u64,
}

impl Key {
    fn new(objectid: u64, item_type: u8, offset: u64) -> Self {
        Key { objectid, item_type, offset }
    }
}

// ---------------------------------------------------------------------------
// Node builders.
// ---------------------------------------------------------------------------

/// Build a single leaf node.  Panics if items don't fit.
fn build_leaf(nodesize: u32, owner: u64, generation: u64, fsid: &[u8; 16], items: &[(Key, Vec<u8>)]) -> Vec<u8> {
    let mut node = vec![0u8; nodesize as usize];

    // Header.
    node[32..48].copy_from_slice(fsid);
    write_le64(&mut node, 48, 0); // bytenr — set by finalize_node
    write_le64(&mut node, 56, 0); // flags
    write_le64(&mut node, 80, generation);
    write_le64(&mut node, 88, owner);
    write_le32(&mut node, 96, items.len() as u32);
    node[100] = 0; // level = 0 (leaf)

    let mut data_end = nodesize as usize;
    for (i, (key, data)) in items.iter().enumerate() {
        data_end -= data.len();

        let item_off = HEADER_SIZE + i * ITEM_SIZE;
        write_key(&mut node[item_off..], key);
        write_le32(&mut node, item_off + KEY_SIZE, data_end as u32);
        write_le32(&mut node, item_off + KEY_SIZE + 4, data.len() as u32);

        node[data_end..data_end + data.len()].copy_from_slice(data);
    }

    node
}

/// Build an internal node from (key, child_logical, generation) entries.
fn build_internal_node(
    nodesize: u32, owner: u64, generation: u64, fsid: &[u8; 16], level: u8,
    children: &[(Key, u64)], // (first_key_in_child, child_logical_addr)
) -> Vec<u8> {
    let mut node = vec![0u8; nodesize as usize];
    node[32..48].copy_from_slice(fsid);
    write_le64(&mut node, 48, 0); // bytenr — set by finalize_node
    write_le64(&mut node, 56, 0); // flags
    write_le64(&mut node, 80, generation);
    write_le64(&mut node, 88, owner);
    write_le32(&mut node, 96, children.len() as u32);
    node[100] = level;

    for (i, (key, blockptr)) in children.iter().enumerate() {
        let kp_off = HEADER_SIZE + i * KEYPTR_SIZE;
        write_key(&mut node[kp_off..], key);
        write_le64(&mut node, kp_off + KEY_SIZE, *blockptr);
        write_le64(&mut node, kp_off + KEY_SIZE + 8, generation);
    }

    node
}

/// Check if items fit in a single leaf.
fn items_fit_in_leaf(nodesize: u32, items: &[(Key, Vec<u8>)]) -> bool {
    let header_and_items = HEADER_SIZE + items.len() * ITEM_SIZE;
    let data_size: usize = items.iter().map(|(_, d)| d.len()).sum();
    header_and_items + data_size <= nodesize as usize
}

/// Split items across multiple leaves, returning a list of (leaf_bytes, first_key).
fn split_into_leaves(
    nodesize: u32, owner: u64, generation: u64, fsid: &[u8; 16],
    items: &[(Key, Vec<u8>)],
) -> Vec<(Vec<u8>, Key)> {
    if items.is_empty() {
        let leaf = build_leaf(nodesize, owner, generation, fsid, &[]);
        return vec![(leaf, Key::new(0, 0, 0))];
    }

    let mut leaves: Vec<(Vec<u8>, Key)> = Vec::new();
    let mut current_items: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut current_data_size: usize = 0;

    for (key, data) in items {
        let new_data_size = current_data_size + data.len();
        let new_item_count = current_items.len() + 1;
        let total_needed = HEADER_SIZE + new_item_count * ITEM_SIZE + new_data_size;

        if total_needed > nodesize as usize && !current_items.is_empty() {
            // Flush current leaf.
            let first_key = current_items[0].0;
            let leaf = build_leaf(nodesize, owner, generation, fsid, &current_items);
            leaves.push((leaf, first_key));
            current_items.clear();
            current_data_size = 0;
        }

        current_data_size += data.len();
        current_items.push((*key, data.clone()));
    }

    // Flush remaining items.
    if !current_items.is_empty() {
        let first_key = current_items[0].0;
        let leaf = build_leaf(nodesize, owner, generation, fsid, &current_items);
        leaves.push((leaf, first_key));
    }

    leaves
}

/// Patch the bytenr field and compute the checksum.
fn finalize_node(node: &mut [u8], logical_addr: u64) {
    write_le64(node, 48, logical_addr);
    let csum = btrfs_crc32c_final(&node[0x20..]);
    node[0..4].copy_from_slice(&csum.to_le_bytes());
    for b in &mut node[4..32] { *b = 0; }
}

// ---------------------------------------------------------------------------
// Item data builders.
// ---------------------------------------------------------------------------

fn make_inode_item(generation: u64, mode: u32, size: u64) -> Vec<u8> {
    let mut buf = vec![0u8; INODE_ITEM_SIZE];
    write_le64(&mut buf, 0, generation);   // generation
    write_le64(&mut buf, 8, generation);   // transid
    write_le64(&mut buf, 16, size);        // size
    write_le64(&mut buf, 24, size);        // nbytes
    write_le64(&mut buf, 32, 0);           // block_group
    write_le32(&mut buf, 40, 1);           // nlink
    write_le32(&mut buf, 44, 0);           // uid
    write_le32(&mut buf, 48, 0);           // gid
    write_le32(&mut buf, 52, mode);        // mode
    // rdev, flags, sequence, reserved, timestamps: all zero.
    buf
}

fn make_inode_ref(name: &str, index: u64) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut buf = vec![0u8; INODE_REF_HEADER_SIZE + name_bytes.len()];
    write_le64(&mut buf, 0, index);
    write_le16(&mut buf, 8, name_bytes.len() as u16);
    buf[INODE_REF_HEADER_SIZE..].copy_from_slice(name_bytes);
    buf
}

fn make_dir_item(child_objectid: u64, file_type: u8, generation: u64, name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut buf = vec![0u8; DIR_ITEM_HEADER_SIZE + name_bytes.len()];
    // location key.
    write_le64(&mut buf, 0, child_objectid);
    buf[8] = BTRFS_INODE_ITEM_KEY;
    write_le64(&mut buf, 9, 0); // offset

    write_le64(&mut buf, 17, generation); // transid
    write_le16(&mut buf, 25, 0);          // data_len
    write_le16(&mut buf, 27, name_bytes.len() as u16);
    buf[29] = file_type;
    buf[DIR_ITEM_HEADER_SIZE..].copy_from_slice(name_bytes);
    buf
}

fn make_root_item(generation: u64, bytenr: u64, level: u8, root_dirid: u64) -> Vec<u8> {
    let mut buf = vec![0u8; ROOT_ITEM_SIZE];
    // Embedded inode item (160 bytes) — mostly zeros.
    let inode = make_inode_item(generation, 0o040755, 0);
    buf[..INODE_ITEM_SIZE].copy_from_slice(&inode);

    let base = INODE_ITEM_SIZE;
    write_le64(&mut buf, base, generation);        // generation
    write_le64(&mut buf, base + 8, root_dirid);    // root_dirid
    write_le64(&mut buf, base + 16, bytenr);       // bytenr
    write_le64(&mut buf, base + 24, 0);            // byte_limit
    write_le64(&mut buf, base + 32, 0);            // bytes_used
    write_le64(&mut buf, base + 40, 0);            // last_snapshot
    write_le64(&mut buf, base + 48, 0);            // flags
    write_le32(&mut buf, base + 56, 1);            // refs
    // drop_progress (17 bytes key), drop_level, level: all zero.
    buf[base + 78] = level;
    write_le64(&mut buf, base + 79, generation);   // generation_v2

    buf
}

fn make_chunk_item(length: u64, chunk_type: u64, physical: u64, dev_uuid: &[u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; CHUNK_ITEM_SIZE + STRIPE_SIZE];
    write_le64(&mut buf, 0, length);          // length
    write_le64(&mut buf, 8, BTRFS_EXTENT_TREE_OBJECTID as u64); // owner
    write_le64(&mut buf, 16, STRIPESIZE as u64); // stripe_len
    write_le64(&mut buf, 24, chunk_type);     // type
    write_le32(&mut buf, 32, SECTORSIZE);     // io_align
    write_le32(&mut buf, 36, SECTORSIZE);     // io_width
    write_le32(&mut buf, 40, SECTORSIZE);     // sector_size
    write_le16(&mut buf, 44, 1);              // num_stripes
    write_le16(&mut buf, 46, 0);              // sub_stripes

    // Stripe.
    let s = CHUNK_ITEM_SIZE;
    write_le64(&mut buf, s, 1);               // devid
    write_le64(&mut buf, s + 8, physical);    // offset
    buf[s + 16..s + 32].copy_from_slice(dev_uuid);

    buf
}

fn make_dev_item_data(devid: u64, total_bytes: u64, bytes_used: u64, dev_uuid: &[u8; 16], fsid: &[u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; DEV_ITEM_SIZE];
    write_le64(&mut buf, 0, devid);
    write_le64(&mut buf, 8, total_bytes);
    write_le64(&mut buf, 16, bytes_used);
    write_le32(&mut buf, 24, SECTORSIZE);    // io_align
    write_le32(&mut buf, 28, SECTORSIZE);    // io_width
    write_le32(&mut buf, 32, SECTORSIZE);    // sector_size
    // type, generation, start_offset, dev_group: zero.
    buf[66..82].copy_from_slice(dev_uuid);
    buf[82..98].copy_from_slice(fsid);
    buf
}

fn make_inline_extent(generation: u64, data: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 21 + data.len()];
    write_le64(&mut buf, 0, generation);
    write_le64(&mut buf, 8, data.len() as u64); // ram_bytes
    buf[16] = 0; // compression
    buf[17] = 0; // encryption
    write_le16(&mut buf, 18, 0); // other_encoding
    buf[20] = BTRFS_FILE_EXTENT_INLINE;
    buf[21..].copy_from_slice(data);
    buf
}

fn make_regular_extent(generation: u64, disk_bytenr: u64, disk_num_bytes: u64, num_bytes: u64) -> Vec<u8> {
    let mut buf = vec![0u8; 53];
    write_le64(&mut buf, 0, generation);
    write_le64(&mut buf, 8, num_bytes); // ram_bytes
    buf[16] = 0; // compression
    buf[17] = 0; // encryption
    write_le16(&mut buf, 18, 0); // other_encoding
    buf[20] = BTRFS_FILE_EXTENT_REG;
    write_le64(&mut buf, 21, disk_bytenr);
    write_le64(&mut buf, 29, disk_num_bytes);
    write_le64(&mut buf, 37, 0); // offset
    write_le64(&mut buf, 45, num_bytes);
    buf
}

fn write_chunk_item_raw(buf: &mut [u8], length: u64, chunk_type: u64) {
    write_le64(buf, 0, length);
    write_le64(buf, 8, BTRFS_EXTENT_TREE_OBJECTID as u64);
    write_le64(buf, 16, STRIPESIZE as u64);
    write_le64(buf, 24, chunk_type);
    write_le32(buf, 32, SECTORSIZE);
    write_le32(buf, 36, SECTORSIZE);
    write_le32(buf, 40, SECTORSIZE);
    write_le16(buf, 44, 1);
    write_le16(buf, 46, 0);
}

fn write_stripe_raw(buf: &mut [u8], devid: u64, physical: u64, dev_uuid: &[u8; 16]) {
    write_le64(buf, 0, devid);
    write_le64(buf, 8, physical);
    buf[16..32].copy_from_slice(dev_uuid);
}

// ---------------------------------------------------------------------------
// Byte-level helpers.
// ---------------------------------------------------------------------------

fn write_key(buf: &mut [u8], key: &Key) {
    write_le64(buf, 0, key.objectid);
    buf[8] = key.item_type;
    write_le64(buf, 9, key.offset);
}

fn write_le16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset+2].copy_from_slice(&val.to_le_bytes());
}

fn write_le32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset+4].copy_from_slice(&val.to_le_bytes());
}

fn write_le64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset+8].copy_from_slice(&val.to_le_bytes());
}

// ---------------------------------------------------------------------------
// CRC32C.
// ---------------------------------------------------------------------------

fn btrfs_crc32c_final(data: &[u8]) -> u32 {
    let raw = crc32c::crc32c(data);
    // crc32c crate returns the finalized CRC.  Btrfs stores ~crc.
    // Wait — let's verify: the crc32c crate applies the standard CRC32C
    // which is: init=!0, xor_out=!0.  Btrfs stores the result of
    // ~crc32c(!0, data), which is the same as the standard CRC32C output.
    // So we can use the crate result directly.
    raw
}

fn btrfs_name_hash(name: &[u8]) -> u64 {
    // ~crc32c(name) — but using the crate which already does ~(init=~0).
    // Actually, btrfs_name_hash is ~crc32c(~0, name) where crc32c here is
    // the RAW accumulator without final XOR.  Since the crate does
    // init=~0 then xor_out=~0, crate_result = ~raw.  So raw = ~crate_result.
    // And btrfs_name_hash = ~raw = crate_result.  But that can't be right
    // because the crate result IS the standard CRC32C which is ~raw...
    //
    // Let me be precise:
    // Standard CRC32C: seed = 0xFFFFFFFF, result = ~accumulator
    // crc32c crate: returns standard CRC32C
    // btrfs_name_hash: !crc32c(!0, name) where crc32c = raw accumulator
    //                = !(raw_accumulator) = standard CRC32C
    //
    // So btrfs_name_hash = crc32c::crc32c(name) !
    // Actually wait, let me re-check the Linux kernel:
    //   static inline u64 btrfs_name_hash(const char *name, int len) {
    //       return crc32c((u32)~1, name, len);
    //   }
    // And Linux crc32c returns the raw accumulator (no final XOR).
    // So btrfs_name_hash = raw_crc32c(~0, name) = ~(standard_crc32c(name))
    //
    // Since the crc32c crate returns standard CRC32C, we need to negate it.
    !crc32c::crc32c(name) as u64
}
