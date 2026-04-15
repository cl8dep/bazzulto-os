// fs/btrfs/ondisk.rs — Btrfs on-disk data structures.
//
// All structures are `#[repr(C, packed)]` because the Btrfs on-disk format
// has no alignment padding.  Fields are little-endian on disk.
//
// Reference: btrfs-progs include/uapi/linux/btrfs_tree.h,
//            https://btrfs.readthedocs.io/en/latest/dev/On-disk-format.html

// ---------------------------------------------------------------------------
// Magic and offsets
// ---------------------------------------------------------------------------

/// Btrfs superblock magic: "_BHRfS_M" as a little-endian u64.
pub const BTRFS_MAGIC: u64 = 0x4D5F53665248425F; // "_BHRfS_M"

/// Byte offset of the primary superblock on the device.
pub const BTRFS_SUPER_INFO_OFFSET: u64 = 0x10000; // 64 KiB

/// Size of the superblock in bytes.
pub const BTRFS_SUPER_INFO_SIZE: usize = 4096;

/// Maximum label length (including NUL terminator).
pub const BTRFS_LABEL_SIZE: usize = 256;

/// Size of the sys_chunk_array inside the superblock.
pub const BTRFS_SYSTEM_CHUNK_ARRAY_SIZE: usize = 2048;

/// Size of the checksum field in headers and the superblock.
pub const BTRFS_CSUM_SIZE: usize = 32;

/// Size of a UUID field.
pub const BTRFS_UUID_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Checksum types
// ---------------------------------------------------------------------------

/// CRC32C (Castagnoli) — the only checksum type we support.
pub const BTRFS_CSUM_TYPE_CRC32C: u16 = 0;

// ---------------------------------------------------------------------------
// Object IDs (well-known)
// ---------------------------------------------------------------------------

/// Root tree.
pub const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;
/// Extent tree (free space / extent allocation).
pub const BTRFS_EXTENT_TREE_OBJECTID: u64 = 2;
/// Chunk tree (logical → physical mapping).
pub const BTRFS_CHUNK_TREE_OBJECTID: u64 = 3;
/// Device tree.
pub const BTRFS_DEV_TREE_OBJECTID: u64 = 4;
/// FS tree (default subvolume).
pub const BTRFS_FS_TREE_OBJECTID: u64 = 5;
/// Root directory objectid inside root tree.
pub const BTRFS_ROOT_TREE_DIR_OBJECTID: u64 = 6;
/// Checksum tree.
pub const BTRFS_CSUM_TREE_OBJECTID: u64 = 7;
/// UUID tree.
pub const BTRFS_UUID_TREE_OBJECTID: u64 = 9;
/// Free space tree.
pub const BTRFS_FREE_SPACE_TREE_OBJECTID: u64 = 10;

/// First free objectid for user files/dirs.
pub const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;
/// Last free objectid.
pub const BTRFS_LAST_FREE_OBJECTID: u64 = u64::MAX - 256;

/// First chunk tree item objectid (always 256 in FIRST_CHUNK_TREE).
pub const BTRFS_FIRST_CHUNK_TREE_OBJECTID: u64 = 256;

/// Dev items key objectid.
pub const BTRFS_DEV_ITEMS_OBJECTID: u64 = 1;

// ---------------------------------------------------------------------------
// Item types (key.item_type values)
// ---------------------------------------------------------------------------

pub const BTRFS_INODE_ITEM_KEY: u8 = 1;
pub const BTRFS_INODE_REF_KEY: u8 = 12;
pub const BTRFS_INODE_EXTREF_KEY: u8 = 13;
pub const BTRFS_DIR_ITEM_KEY: u8 = 84;
pub const BTRFS_DIR_INDEX_KEY: u8 = 96;
pub const BTRFS_EXTENT_DATA_KEY: u8 = 108;
pub const BTRFS_EXTENT_ITEM_KEY: u8 = 168;
pub const BTRFS_METADATA_ITEM_KEY: u8 = 169;
pub const BTRFS_BLOCK_GROUP_ITEM_KEY: u8 = 192;
pub const BTRFS_DEV_ITEM_KEY: u8 = 216;
pub const BTRFS_CHUNK_ITEM_KEY: u8 = 228;
pub const BTRFS_ROOT_ITEM_KEY: u8 = 132;
pub const BTRFS_ROOT_REF_KEY: u8 = 156;
pub const BTRFS_ROOT_BACKREF_KEY: u8 = 144;
pub const BTRFS_FREE_SPACE_INFO_KEY: u8 = 198;
pub const BTRFS_FREE_SPACE_EXTENT_KEY: u8 = 199;

// ---------------------------------------------------------------------------
// Inode flags and mode constants
// ---------------------------------------------------------------------------

/// File type bits in btrfs_dir_item.dir_type (same as Linux d_type).
pub const BTRFS_FT_REG_FILE: u8 = 1;
pub const BTRFS_FT_DIR: u8 = 2;
pub const BTRFS_FT_CHRDEV: u8 = 3;
pub const BTRFS_FT_BLKDEV: u8 = 4;
pub const BTRFS_FT_FIFO: u8 = 5;
pub const BTRFS_FT_SOCK: u8 = 6;
pub const BTRFS_FT_SYMLINK: u8 = 7;

/// Extent data types (btrfs_file_extent_item.type).
pub const BTRFS_FILE_EXTENT_INLINE: u8 = 0;
pub const BTRFS_FILE_EXTENT_REG: u8 = 1;
pub const BTRFS_FILE_EXTENT_PREALLOC: u8 = 2;

// ---------------------------------------------------------------------------
// Block group / chunk flags
// ---------------------------------------------------------------------------

pub const BTRFS_BLOCK_GROUP_DATA: u64 = 1 << 0;
pub const BTRFS_BLOCK_GROUP_SYSTEM: u64 = 1 << 1;
pub const BTRFS_BLOCK_GROUP_METADATA: u64 = 1 << 2;

/// Single profile (no RAID).
pub const BTRFS_BLOCK_GROUP_SINGLE: u64 = 0;

// ---------------------------------------------------------------------------
// Incompat feature flags
// ---------------------------------------------------------------------------

pub const BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;
pub const BTRFS_FEATURE_INCOMPAT_DEFAULT_SUBVOL: u64 = 1 << 1;
pub const BTRFS_FEATURE_INCOMPAT_MIXED_GROUPS: u64 = 1 << 2;
pub const BTRFS_FEATURE_INCOMPAT_COMPRESS_LZO: u64 = 1 << 3;
pub const BTRFS_FEATURE_INCOMPAT_COMPRESS_ZSTD: u64 = 1 << 4;
pub const BTRFS_FEATURE_INCOMPAT_BIG_METADATA: u64 = 1 << 5;
pub const BTRFS_FEATURE_INCOMPAT_EXTENDED_IREF: u64 = 1 << 6;
pub const BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;
pub const BTRFS_FEATURE_INCOMPAT_NO_HOLES: u64 = 1 << 9;

// ---------------------------------------------------------------------------
// Key — the universal sort key for all B-tree items.
// ---------------------------------------------------------------------------

/// A Btrfs disk key: (objectid, type, offset).
///
/// Keys are sorted lexicographically: first by objectid, then by type, then
/// by offset.  This ordering determines the physical layout of all items in
/// every B-tree.
///
/// Size: 17 bytes on disk.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C, packed)]
pub struct BtrfsKey {
    pub objectid: u64,
    pub item_type: u8,
    pub offset: u64,
}

impl BtrfsKey {
    pub const SIZE: usize = 17;

    pub const fn new(objectid: u64, item_type: u8, offset: u64) -> Self {
        BtrfsKey { objectid, item_type, offset }
    }

    /// Zero key — sorts before all real keys.
    pub const ZERO: BtrfsKey = BtrfsKey { objectid: 0, item_type: 0, offset: 0 };

    /// Parse from a byte slice (must be >= 17 bytes).
    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsKey {
            objectid: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            item_type: buf[8],
            offset: u64::from_le_bytes(buf[9..17].try_into().unwrap()),
        }
    }

    /// Serialize to a byte buffer (must be >= 17 bytes).
    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.objectid.to_le_bytes());
        buf[8] = self.item_type;
        buf[9..17].copy_from_slice(&self.offset.to_le_bytes());
    }
}

impl PartialOrd for BtrfsKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BtrfsKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Copy fields to locals to avoid unaligned references to packed struct.
        // Fields are already in native byte order (from_le_bytes in from_bytes,
        // or native values from BtrfsKey::new).
        let a_oid = self.objectid;
        let b_oid = other.objectid;
        match a_oid.cmp(&b_oid) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        let a_ty = self.item_type;
        let b_ty = other.item_type;
        match a_ty.cmp(&b_ty) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        let a_off = self.offset;
        let b_off = other.offset;
        a_off.cmp(&b_off)
    }
}

impl core::fmt::Debug for BtrfsKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Key({}, {}, {})",
            u64::from_le(self.objectid),
            self.item_type,
            u64::from_le(self.offset))
    }
}

// ---------------------------------------------------------------------------
// B-tree node header
// ---------------------------------------------------------------------------

/// Header present at the start of every B-tree node (internal or leaf).
///
/// Size: 101 bytes on disk.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct BtrfsHeader {
    /// Checksum of everything after this field.
    pub csum: [u8; BTRFS_CSUM_SIZE],
    /// Filesystem UUID.
    pub fsid: [u8; BTRFS_UUID_SIZE],
    /// Logical byte number of this node (used for CoW verification).
    pub bytenr: u64,
    /// Flags.
    pub flags: u64,
    /// UUID of the chunk tree that owns this node.
    pub chunk_tree_uuid: [u8; BTRFS_UUID_SIZE],
    /// Transaction generation that created this node.
    pub generation: u64,
    /// Tree that owns this node (objectid of the root item in root tree).
    pub owner: u64,
    /// Number of items (key/pointer pairs for internal, key/data for leaf).
    pub nritems: u32,
    /// Level in the tree: 0 = leaf, >0 = internal.
    pub level: u8,
}

impl BtrfsHeader {
    pub const SIZE: usize = 101;

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut header = BtrfsHeader {
            csum: [0u8; BTRFS_CSUM_SIZE],
            fsid: [0u8; BTRFS_UUID_SIZE],
            bytenr: 0,
            flags: 0,
            chunk_tree_uuid: [0u8; BTRFS_UUID_SIZE],
            generation: 0,
            owner: 0,
            nritems: 0,
            level: 0,
        };
        header.csum.copy_from_slice(&buf[0..32]);
        header.fsid.copy_from_slice(&buf[32..48]);
        header.bytenr = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        header.flags = u64::from_le_bytes(buf[56..64].try_into().unwrap());
        header.chunk_tree_uuid.copy_from_slice(&buf[64..80]);
        header.generation = u64::from_le_bytes(buf[80..88].try_into().unwrap());
        header.owner = u64::from_le_bytes(buf[88..96].try_into().unwrap());
        header.nritems = u32::from_le_bytes(buf[96..100].try_into().unwrap());
        header.level = buf[100];
        header
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..32].copy_from_slice(&self.csum);
        buf[32..48].copy_from_slice(&self.fsid);
        buf[48..56].copy_from_slice(&self.bytenr.to_le_bytes());
        buf[56..64].copy_from_slice(&self.flags.to_le_bytes());
        buf[64..80].copy_from_slice(&self.chunk_tree_uuid);
        buf[80..88].copy_from_slice(&self.generation.to_le_bytes());
        buf[88..96].copy_from_slice(&self.owner.to_le_bytes());
        buf[96..100].copy_from_slice(&self.nritems.to_le_bytes());
        buf[100] = self.level;
    }
}

// ---------------------------------------------------------------------------
// Leaf item pointer
// ---------------------------------------------------------------------------

/// An item inside a leaf node: key + (offset, size) pointing into the leaf's
/// data area.
///
/// Size: 25 bytes on disk (17 key + 4 offset + 4 size).
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct BtrfsItem {
    pub key: BtrfsKey,
    /// Byte offset of the item data, relative to the end of the header.
    /// NOTE: in Btrfs, data_offset is relative to the end of the fixed item
    /// array area — i.e. relative to `(header_size + nritems * item_size)`.
    /// Actually, it's relative to the start of the *leaf data area*, which
    /// grows backwards from the end of the node.  The offset is measured from
    /// byte 0 of the node (not from the end of the header).
    pub offset: u32,
    /// Size of the item data in bytes.
    pub size: u32,
}

impl BtrfsItem {
    pub const SIZE: usize = 25;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsItem {
            key: BtrfsKey::from_bytes(&buf[0..17]),
            offset: u32::from_le_bytes(buf[17..21].try_into().unwrap()),
            size: u32::from_le_bytes(buf[21..25].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        self.key.to_bytes(&mut buf[0..17]);
        buf[17..21].copy_from_slice(&self.offset.to_le_bytes());
        buf[21..25].copy_from_slice(&self.size.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Internal node key/pointer pair
// ---------------------------------------------------------------------------

/// A key/pointer inside an internal (non-leaf) node.
///
/// Size: 33 bytes on disk (17 key + 8 blockptr + 8 generation).
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct BtrfsKeyPtr {
    pub key: BtrfsKey,
    /// Logical byte address of the child node.
    pub blockptr: u64,
    /// Generation of the child node (for CoW validation).
    pub generation: u64,
}

impl BtrfsKeyPtr {
    pub const SIZE: usize = 33;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsKeyPtr {
            key: BtrfsKey::from_bytes(&buf[0..17]),
            blockptr: u64::from_le_bytes(buf[17..25].try_into().unwrap()),
            generation: u64::from_le_bytes(buf[25..33].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        self.key.to_bytes(&mut buf[0..17]);
        buf[17..25].copy_from_slice(&self.blockptr.to_le_bytes());
        buf[25..33].copy_from_slice(&self.generation.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Superblock (partial — we parse fields manually from raw bytes)
// ---------------------------------------------------------------------------

/// Parsed superblock fields (not a direct on-disk struct due to odd alignment
/// and the embedded dev_item / sys_chunk_array).
pub struct BtrfsSuperblock {
    pub csum: [u8; BTRFS_CSUM_SIZE],
    pub fsid: [u8; BTRFS_UUID_SIZE],
    pub bytenr: u64,
    pub flags: u64,
    pub magic: u64,
    pub generation: u64,
    pub root: u64,
    pub chunk_root: u64,
    pub log_root: u64,
    pub total_bytes: u64,
    pub bytes_used: u64,
    pub root_dir_objectid: u64,
    pub num_devices: u64,
    pub sectorsize: u32,
    pub nodesize: u32,
    pub stripesize: u32,
    pub sys_chunk_array_size: u32,
    pub chunk_root_generation: u64,
    pub compat_flags: u64,
    pub compat_ro_flags: u64,
    pub incompat_flags: u64,
    pub csum_type: u16,
    pub root_level: u8,
    pub chunk_root_level: u8,
    pub log_root_level: u8,
    pub dev_item: BtrfsDevItem,
    pub label: [u8; BTRFS_LABEL_SIZE],
    pub cache_generation: u64,
    pub uuid_tree_generation: u64,
    pub metadata_uuid: [u8; BTRFS_UUID_SIZE],
    /// Raw sys_chunk_array bytes (parsed separately by chunk.rs).
    pub sys_chunk_array: [u8; BTRFS_SYSTEM_CHUNK_ARRAY_SIZE],
}

// Superblock field offsets (from btrfs_tree.h).
pub const SB_OFF_CSUM: usize = 0x00;
pub const SB_OFF_FSID: usize = 0x20;
pub const SB_OFF_BYTENR: usize = 0x30;
pub const SB_OFF_FLAGS: usize = 0x38;
pub const SB_OFF_MAGIC: usize = 0x40;
pub const SB_OFF_GENERATION: usize = 0x48;
pub const SB_OFF_ROOT: usize = 0x50;
pub const SB_OFF_CHUNK_ROOT: usize = 0x58;
pub const SB_OFF_LOG_ROOT: usize = 0x60;
pub const SB_OFF_TOTAL_BYTES: usize = 0x70;
pub const SB_OFF_BYTES_USED: usize = 0x78;
pub const SB_OFF_ROOT_DIR_OBJECTID: usize = 0x80;
pub const SB_OFF_NUM_DEVICES: usize = 0x88;
pub const SB_OFF_SECTORSIZE: usize = 0x90;
pub const SB_OFF_NODESIZE: usize = 0x94;
pub const SB_OFF_STRIPESIZE: usize = 0x9C;
pub const SB_OFF_SYS_CHUNK_ARRAY_SIZE: usize = 0xA0;
pub const SB_OFF_CHUNK_ROOT_GENERATION: usize = 0xA4;
pub const SB_OFF_COMPAT_FLAGS: usize = 0xAC;
pub const SB_OFF_COMPAT_RO_FLAGS: usize = 0xB4;
pub const SB_OFF_INCOMPAT_FLAGS: usize = 0xBC;
pub const SB_OFF_CSUM_TYPE: usize = 0xC4;
pub const SB_OFF_ROOT_LEVEL: usize = 0xC6;
pub const SB_OFF_CHUNK_ROOT_LEVEL: usize = 0xC7;
pub const SB_OFF_LOG_ROOT_LEVEL: usize = 0xC8;
pub const SB_OFF_DEV_ITEM: usize = 0xC9;
pub const SB_OFF_LABEL: usize = 0x12B;
pub const SB_OFF_CACHE_GENERATION: usize = 0x22B;
pub const SB_OFF_UUID_TREE_GENERATION: usize = 0x233;
pub const SB_OFF_METADATA_UUID: usize = 0x23B;
pub const SB_OFF_SYS_CHUNK_ARRAY: usize = 0x2FB;

// ---------------------------------------------------------------------------
// Dev item
// ---------------------------------------------------------------------------

/// On-disk device item (embedded in superblock and stored in dev tree).
///
/// Size: 98 bytes.
#[derive(Clone, Copy)]
pub struct BtrfsDevItem {
    pub devid: u64,
    pub total_bytes: u64,
    pub bytes_used: u64,
    pub io_align: u32,
    pub io_width: u32,
    pub sector_size: u32,
    pub dev_type: u64,
    pub generation: u64,
    pub start_offset: u64,
    pub dev_group: u32,
    pub seek_speed: u8,
    pub bandwidth: u8,
    pub uuid: [u8; BTRFS_UUID_SIZE],
    pub fsid: [u8; BTRFS_UUID_SIZE],
}

impl BtrfsDevItem {
    pub const SIZE: usize = 98;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsDevItem {
            devid: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            total_bytes: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            bytes_used: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            io_align: u32::from_le_bytes(buf[24..28].try_into().unwrap()),
            io_width: u32::from_le_bytes(buf[28..32].try_into().unwrap()),
            sector_size: u32::from_le_bytes(buf[32..36].try_into().unwrap()),
            dev_type: u64::from_le_bytes(buf[36..44].try_into().unwrap()),
            generation: u64::from_le_bytes(buf[44..52].try_into().unwrap()),
            start_offset: u64::from_le_bytes(buf[52..60].try_into().unwrap()),
            dev_group: u32::from_le_bytes(buf[60..64].try_into().unwrap()),
            seek_speed: buf[64],
            bandwidth: buf[65],
            uuid: {
                let mut u = [0u8; 16];
                u.copy_from_slice(&buf[66..82]);
                u
            },
            fsid: {
                let mut f = [0u8; 16];
                f.copy_from_slice(&buf[82..98]);
                f
            },
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.devid.to_le_bytes());
        buf[8..16].copy_from_slice(&self.total_bytes.to_le_bytes());
        buf[16..24].copy_from_slice(&self.bytes_used.to_le_bytes());
        buf[24..28].copy_from_slice(&self.io_align.to_le_bytes());
        buf[28..32].copy_from_slice(&self.io_width.to_le_bytes());
        buf[32..36].copy_from_slice(&self.sector_size.to_le_bytes());
        buf[36..44].copy_from_slice(&self.dev_type.to_le_bytes());
        buf[44..52].copy_from_slice(&self.generation.to_le_bytes());
        buf[52..60].copy_from_slice(&self.start_offset.to_le_bytes());
        buf[60..64].copy_from_slice(&self.dev_group.to_le_bytes());
        buf[64] = self.seek_speed;
        buf[65] = self.bandwidth;
        buf[66..82].copy_from_slice(&self.uuid);
        buf[82..98].copy_from_slice(&self.fsid);
    }
}

// ---------------------------------------------------------------------------
// Chunk item (stored in chunk tree and sys_chunk_array)
// ---------------------------------------------------------------------------

/// On-disk chunk item header — describes a mapping from logical to physical.
///
/// Followed by `num_stripes` × `BtrfsStripe` entries.
///
/// Size: 80 bytes (header only, without stripes).
#[derive(Clone, Copy)]
pub struct BtrfsChunkItem {
    /// Size of this chunk in bytes.
    pub length: u64,
    /// Owner tree (usually BTRFS_EXTENT_TREE_OBJECTID).
    pub owner: u64,
    /// Stripe length in bytes.
    pub stripe_len: u64,
    /// Block group flags (DATA, METADATA, SYSTEM, profile).
    pub chunk_type: u64,
    /// I/O alignment.
    pub io_align: u32,
    /// I/O width.
    pub io_width: u32,
    /// Sector size.
    pub sector_size: u32,
    /// Number of stripes following this header.
    pub num_stripes: u16,
    /// Sub-stripes (for RAID10).
    pub sub_stripes: u16,
}

impl BtrfsChunkItem {
    pub const SIZE: usize = 48;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsChunkItem {
            length: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            owner: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            stripe_len: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            chunk_type: u64::from_le_bytes(buf[24..32].try_into().unwrap()),
            io_align: u32::from_le_bytes(buf[32..36].try_into().unwrap()),
            io_width: u32::from_le_bytes(buf[36..40].try_into().unwrap()),
            sector_size: u32::from_le_bytes(buf[40..44].try_into().unwrap()),
            num_stripes: u16::from_le_bytes(buf[44..46].try_into().unwrap()),
            sub_stripes: u16::from_le_bytes(buf[46..48].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.length.to_le_bytes());
        buf[8..16].copy_from_slice(&self.owner.to_le_bytes());
        buf[16..24].copy_from_slice(&self.stripe_len.to_le_bytes());
        buf[24..32].copy_from_slice(&self.chunk_type.to_le_bytes());
        buf[32..36].copy_from_slice(&self.io_align.to_le_bytes());
        buf[36..40].copy_from_slice(&self.io_width.to_le_bytes());
        buf[40..44].copy_from_slice(&self.sector_size.to_le_bytes());
        buf[44..46].copy_from_slice(&self.num_stripes.to_le_bytes());
        buf[46..48].copy_from_slice(&self.sub_stripes.to_le_bytes());
    }
}

/// A single stripe within a chunk item.
///
/// Size: 32 bytes.
#[derive(Clone, Copy)]
pub struct BtrfsStripe {
    pub devid: u64,
    pub offset: u64,
    pub dev_uuid: [u8; BTRFS_UUID_SIZE],
}

impl BtrfsStripe {
    pub const SIZE: usize = 32;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsStripe {
            devid: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            offset: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            dev_uuid: {
                let mut u = [0u8; 16];
                u.copy_from_slice(&buf[16..32]);
                u
            },
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.devid.to_le_bytes());
        buf[8..16].copy_from_slice(&self.offset.to_le_bytes());
        buf[16..32].copy_from_slice(&self.dev_uuid);
    }
}

// ---------------------------------------------------------------------------
// Root item (stored in root tree — describes a sub-tree root)
// ---------------------------------------------------------------------------

/// On-disk root item: metadata for a tree root (FS tree, extent tree, etc.).
///
/// Size: 439 bytes.
#[derive(Clone)]
pub struct BtrfsRootItem {
    pub inode: BtrfsInodeItem,
    pub generation: u64,
    pub root_dirid: u64,
    pub bytenr: u64,
    pub byte_limit: u64,
    pub bytes_used: u64,
    pub last_snapshot: u64,
    pub flags: u64,
    pub refs_count: u32,
    pub drop_progress: BtrfsKey,
    pub drop_level: u8,
    pub level: u8,
    pub generation_v2: u64,
    pub uuid: [u8; BTRFS_UUID_SIZE],
    pub parent_uuid: [u8; BTRFS_UUID_SIZE],
    pub received_uuid: [u8; BTRFS_UUID_SIZE],
    pub ctransid: u64,
    pub otransid: u64,
    pub stransid: u64,
    pub rtransid: u64,
    pub ctime: BtrfsTimespec,
    pub otime: BtrfsTimespec,
    pub stime: BtrfsTimespec,
    pub rtime: BtrfsTimespec,
}

impl BtrfsRootItem {
    pub const SIZE: usize = 439;

    pub fn from_bytes(buf: &[u8]) -> Self {
        let inode = BtrfsInodeItem::from_bytes(&buf[0..160]);
        let base = 160;
        BtrfsRootItem {
            inode,
            generation: u64::from_le_bytes(buf[base..base+8].try_into().unwrap()),
            root_dirid: u64::from_le_bytes(buf[base+8..base+16].try_into().unwrap()),
            bytenr: u64::from_le_bytes(buf[base+16..base+24].try_into().unwrap()),
            byte_limit: u64::from_le_bytes(buf[base+24..base+32].try_into().unwrap()),
            bytes_used: u64::from_le_bytes(buf[base+32..base+40].try_into().unwrap()),
            last_snapshot: u64::from_le_bytes(buf[base+40..base+48].try_into().unwrap()),
            flags: u64::from_le_bytes(buf[base+48..base+56].try_into().unwrap()),
            refs_count: u32::from_le_bytes(buf[base+56..base+60].try_into().unwrap()),
            drop_progress: BtrfsKey::from_bytes(&buf[base+60..base+77]),
            drop_level: buf[base+77],
            level: buf[base+78],
            generation_v2: u64::from_le_bytes(buf[base+79..base+87].try_into().unwrap()),
            uuid: { let mut u = [0u8; 16]; u.copy_from_slice(&buf[base+87..base+103]); u },
            parent_uuid: { let mut u = [0u8; 16]; u.copy_from_slice(&buf[base+103..base+119]); u },
            received_uuid: { let mut u = [0u8; 16]; u.copy_from_slice(&buf[base+119..base+135]); u },
            ctransid: u64::from_le_bytes(buf[base+135..base+143].try_into().unwrap()),
            otransid: u64::from_le_bytes(buf[base+143..base+151].try_into().unwrap()),
            stransid: u64::from_le_bytes(buf[base+151..base+159].try_into().unwrap()),
            rtransid: u64::from_le_bytes(buf[base+159..base+167].try_into().unwrap()),
            ctime: BtrfsTimespec::from_bytes(&buf[base+167..base+179]),
            otime: BtrfsTimespec::from_bytes(&buf[base+179..base+191]),
            stime: BtrfsTimespec::from_bytes(&buf[base+191..base+203]),
            rtime: BtrfsTimespec::from_bytes(&buf[base+203..base+215]),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        self.inode.to_bytes(&mut buf[0..160]);
        let base = 160;
        buf[base..base+8].copy_from_slice(&self.generation.to_le_bytes());
        buf[base+8..base+16].copy_from_slice(&self.root_dirid.to_le_bytes());
        buf[base+16..base+24].copy_from_slice(&self.bytenr.to_le_bytes());
        buf[base+24..base+32].copy_from_slice(&self.byte_limit.to_le_bytes());
        buf[base+32..base+40].copy_from_slice(&self.bytes_used.to_le_bytes());
        buf[base+40..base+48].copy_from_slice(&self.last_snapshot.to_le_bytes());
        buf[base+48..base+56].copy_from_slice(&self.flags.to_le_bytes());
        buf[base+56..base+60].copy_from_slice(&self.refs_count.to_le_bytes());
        self.drop_progress.to_bytes(&mut buf[base+60..base+77]);
        buf[base+77] = self.drop_level;
        buf[base+78] = self.level;
        buf[base+79..base+87].copy_from_slice(&self.generation_v2.to_le_bytes());
        buf[base+87..base+103].copy_from_slice(&self.uuid);
        buf[base+103..base+119].copy_from_slice(&self.parent_uuid);
        buf[base+119..base+135].copy_from_slice(&self.received_uuid);
        buf[base+135..base+143].copy_from_slice(&self.ctransid.to_le_bytes());
        buf[base+143..base+151].copy_from_slice(&self.otransid.to_le_bytes());
        buf[base+151..base+159].copy_from_slice(&self.stransid.to_le_bytes());
        buf[base+159..base+167].copy_from_slice(&self.rtransid.to_le_bytes());
        self.ctime.to_bytes(&mut buf[base+167..base+179]);
        self.otime.to_bytes(&mut buf[base+179..base+191]);
        self.stime.to_bytes(&mut buf[base+191..base+203]);
        self.rtime.to_bytes(&mut buf[base+203..base+215]);
    }
}

// ---------------------------------------------------------------------------
// Inode item
// ---------------------------------------------------------------------------

/// On-disk inode item — metadata for a file or directory.
///
/// Size: 160 bytes.
#[derive(Clone, Copy)]
pub struct BtrfsInodeItem {
    pub generation: u64,
    pub transid: u64,
    pub size: u64,
    pub nbytes: u64,
    pub block_group: u64,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub rdev: u64,
    pub flags: u64,
    pub sequence: u64,
    pub reserved: [u64; 4],
    pub atime: BtrfsTimespec,
    pub ctime: BtrfsTimespec,
    pub mtime: BtrfsTimespec,
    pub otime: BtrfsTimespec,
}

impl BtrfsInodeItem {
    pub const SIZE: usize = 160;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsInodeItem {
            generation: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            transid: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            size: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            nbytes: u64::from_le_bytes(buf[24..32].try_into().unwrap()),
            block_group: u64::from_le_bytes(buf[32..40].try_into().unwrap()),
            nlink: u32::from_le_bytes(buf[40..44].try_into().unwrap()),
            uid: u32::from_le_bytes(buf[44..48].try_into().unwrap()),
            gid: u32::from_le_bytes(buf[48..52].try_into().unwrap()),
            mode: u32::from_le_bytes(buf[52..56].try_into().unwrap()),
            rdev: u64::from_le_bytes(buf[56..64].try_into().unwrap()),
            flags: u64::from_le_bytes(buf[64..72].try_into().unwrap()),
            sequence: u64::from_le_bytes(buf[72..80].try_into().unwrap()),
            reserved: [
                u64::from_le_bytes(buf[80..88].try_into().unwrap()),
                u64::from_le_bytes(buf[88..96].try_into().unwrap()),
                u64::from_le_bytes(buf[96..104].try_into().unwrap()),
                u64::from_le_bytes(buf[104..112].try_into().unwrap()),
            ],
            atime: BtrfsTimespec::from_bytes(&buf[112..124]),
            ctime: BtrfsTimespec::from_bytes(&buf[124..136]),
            mtime: BtrfsTimespec::from_bytes(&buf[136..148]),
            otime: BtrfsTimespec::from_bytes(&buf[148..160]),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.generation.to_le_bytes());
        buf[8..16].copy_from_slice(&self.transid.to_le_bytes());
        buf[16..24].copy_from_slice(&self.size.to_le_bytes());
        buf[24..32].copy_from_slice(&self.nbytes.to_le_bytes());
        buf[32..40].copy_from_slice(&self.block_group.to_le_bytes());
        buf[40..44].copy_from_slice(&self.nlink.to_le_bytes());
        buf[44..48].copy_from_slice(&self.uid.to_le_bytes());
        buf[48..52].copy_from_slice(&self.gid.to_le_bytes());
        buf[52..56].copy_from_slice(&self.mode.to_le_bytes());
        buf[56..64].copy_from_slice(&self.rdev.to_le_bytes());
        buf[64..72].copy_from_slice(&self.flags.to_le_bytes());
        buf[72..80].copy_from_slice(&self.sequence.to_le_bytes());
        for (i, &val) in self.reserved.iter().enumerate() {
            let off = 80 + i * 8;
            buf[off..off+8].copy_from_slice(&val.to_le_bytes());
        }
        self.atime.to_bytes(&mut buf[112..124]);
        self.ctime.to_bytes(&mut buf[124..136]);
        self.mtime.to_bytes(&mut buf[136..148]);
        self.otime.to_bytes(&mut buf[148..160]);
    }

    /// Return true if mode indicates a directory (S_IFDIR = 0o40000).
    pub fn is_directory(&self) -> bool {
        (self.mode & 0o170000) == 0o040000
    }

    /// Return true if mode indicates a regular file (S_IFMT = 0o100000).
    pub fn is_regular_file(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }

    /// Return true if mode indicates a symlink (S_IFLNK = 0o120000).
    pub fn is_symlink(&self) -> bool {
        (self.mode & 0o170000) == 0o120000
    }
}

// ---------------------------------------------------------------------------
// Timespec
// ---------------------------------------------------------------------------

/// On-disk timespec — seconds + nanoseconds.
///
/// Size: 12 bytes.
#[derive(Clone, Copy, Default)]
pub struct BtrfsTimespec {
    pub sec: u64,
    pub nsec: u32,
}

impl BtrfsTimespec {
    pub const SIZE: usize = 12;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsTimespec {
            sec: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            nsec: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.sec.to_le_bytes());
        buf[8..12].copy_from_slice(&self.nsec.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Inode ref (links an inode to its parent directory)
// ---------------------------------------------------------------------------

/// On-disk inode reference — stored in the FS tree under key
/// (child_objectid, INODE_REF, parent_objectid).
///
/// Variable size: 8 bytes header + name_len bytes.
pub struct BtrfsInodeRef {
    /// Directory entry index (DIR_INDEX offset) for this name.
    pub index: u64,
    /// Length of the name that follows.
    pub name_len: u16,
    // Followed by `name_len` bytes of the filename.
}

impl BtrfsInodeRef {
    pub const HEADER_SIZE: usize = 10;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsInodeRef {
            index: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            name_len: u16::from_le_bytes(buf[8..10].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.index.to_le_bytes());
        buf[8..10].copy_from_slice(&self.name_len.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Dir item
// ---------------------------------------------------------------------------

/// On-disk directory item — stored under key
/// (parent_objectid, DIR_ITEM, name_hash) or
/// (parent_objectid, DIR_INDEX, sequence).
///
/// Variable size: 30 bytes header + name_len bytes.
pub struct BtrfsDirItem {
    pub location: BtrfsKey,
    pub transid: u64,
    pub data_len: u16,
    pub name_len: u16,
    pub dir_type: u8,
    // Followed by `name_len` bytes of the name, then `data_len` bytes of data.
}

impl BtrfsDirItem {
    pub const HEADER_SIZE: usize = 30;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsDirItem {
            location: BtrfsKey::from_bytes(&buf[0..17]),
            transid: u64::from_le_bytes(buf[17..25].try_into().unwrap()),
            data_len: u16::from_le_bytes(buf[25..27].try_into().unwrap()),
            name_len: u16::from_le_bytes(buf[27..29].try_into().unwrap()),
            dir_type: buf[29],
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        self.location.to_bytes(&mut buf[0..17]);
        buf[17..25].copy_from_slice(&self.transid.to_le_bytes());
        buf[25..27].copy_from_slice(&self.data_len.to_le_bytes());
        buf[27..29].copy_from_slice(&self.name_len.to_le_bytes());
        buf[29] = self.dir_type;
    }
}

// ---------------------------------------------------------------------------
// File extent item
// ---------------------------------------------------------------------------

/// On-disk file extent item — describes file data for a range of bytes.
///
/// Size: 21 bytes for inline, 53 bytes for regular/prealloc.
pub struct BtrfsFileExtentItem {
    pub generation: u64,
    /// Size of decoded (decompressed) data.
    pub ram_bytes: u64,
    /// Compression type (0 = none).
    pub compression: u8,
    /// Encryption type (0 = none, reserved).
    pub encryption: u8,
    /// Other encoding (0 = none, reserved).
    pub other_encoding: u16,
    /// Type: 0 = inline, 1 = regular, 2 = prealloc.
    pub extent_type: u8,
    // --- Fields below only present for regular/prealloc extents ---
    /// Logical address of the extent on disk (0 for holes).
    pub disk_bytenr: u64,
    /// Size of the extent on disk (may differ if compressed).
    pub disk_num_bytes: u64,
    /// Offset within the extent (for sharing extents across files).
    pub offset: u64,
    /// Number of bytes referenced from this file.
    pub num_bytes: u64,
}

impl BtrfsFileExtentItem {
    pub const INLINE_HEADER_SIZE: usize = 21;
    pub const REG_SIZE: usize = 53;

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut item = BtrfsFileExtentItem {
            generation: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            ram_bytes: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            compression: buf[16],
            encryption: buf[17],
            other_encoding: u16::from_le_bytes(buf[18..20].try_into().unwrap()),
            extent_type: buf[20],
            disk_bytenr: 0,
            disk_num_bytes: 0,
            offset: 0,
            num_bytes: 0,
        };
        if item.extent_type != BTRFS_FILE_EXTENT_INLINE && buf.len() >= Self::REG_SIZE {
            item.disk_bytenr = u64::from_le_bytes(buf[21..29].try_into().unwrap());
            item.disk_num_bytes = u64::from_le_bytes(buf[29..37].try_into().unwrap());
            item.offset = u64::from_le_bytes(buf[37..45].try_into().unwrap());
            item.num_bytes = u64::from_le_bytes(buf[45..53].try_into().unwrap());
        }
        item
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.generation.to_le_bytes());
        buf[8..16].copy_from_slice(&self.ram_bytes.to_le_bytes());
        buf[16] = self.compression;
        buf[17] = self.encryption;
        buf[18..20].copy_from_slice(&self.other_encoding.to_le_bytes());
        buf[20] = self.extent_type;
        if self.extent_type != BTRFS_FILE_EXTENT_INLINE && buf.len() >= Self::REG_SIZE {
            buf[21..29].copy_from_slice(&self.disk_bytenr.to_le_bytes());
            buf[29..37].copy_from_slice(&self.disk_num_bytes.to_le_bytes());
            buf[37..45].copy_from_slice(&self.offset.to_le_bytes());
            buf[45..53].copy_from_slice(&self.num_bytes.to_le_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Extent item (extent tree)
// ---------------------------------------------------------------------------

/// On-disk extent item — metadata for a used extent.
///
/// Size: 24 bytes.
pub struct BtrfsExtentItem {
    pub refs_count: u64,
    pub generation: u64,
    pub flags: u64,
}

impl BtrfsExtentItem {
    pub const SIZE: usize = 24;

    /// Extent flags.
    pub const FLAG_DATA: u64 = 1;
    pub const FLAG_TREE_BLOCK: u64 = 2;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsExtentItem {
            refs_count: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            generation: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            flags: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.refs_count.to_le_bytes());
        buf[8..16].copy_from_slice(&self.generation.to_le_bytes());
        buf[16..24].copy_from_slice(&self.flags.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Block group item (extent tree)
// ---------------------------------------------------------------------------

/// On-disk block group item — describes a chunk's allocation state.
///
/// Size: 24 bytes.
pub struct BtrfsBlockGroupItem {
    pub used: u64,
    pub chunk_objectid: u64,
    pub flags: u64,
}

impl BtrfsBlockGroupItem {
    pub const SIZE: usize = 24;

    pub fn from_bytes(buf: &[u8]) -> Self {
        BtrfsBlockGroupItem {
            used: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            chunk_objectid: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            flags: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        buf[0..8].copy_from_slice(&self.used.to_le_bytes());
        buf[8..16].copy_from_slice(&self.chunk_objectid.to_le_bytes());
        buf[16..24].copy_from_slice(&self.flags.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Utility: name hash (CRC32C of filename, used as DIR_ITEM key offset)
// ---------------------------------------------------------------------------

/// Compute the Btrfs directory name hash.
///
/// This is `crc32c(~0, name)` — the raw CRC32C accumulator, same as
/// `btrfs_name_hash()` in the Linux kernel (fs/btrfs/hash.h).
/// Note: NO final bitwise NOT — Btrfs uses the raw accumulator, not the
/// standard CRC32C value.
pub fn btrfs_name_hash(name: &[u8]) -> u64 {
    super::crc32c::crc32c(!0u32, name) as u64
}
