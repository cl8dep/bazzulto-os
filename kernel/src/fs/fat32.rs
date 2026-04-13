// fs/fat32.rs — FAT32 filesystem driver.
//
// Port of kernel/filesystem/filesystem_disk.c.
// Uses hal::disk::read_sectors / write_sectors for all I/O.
// Supports: init, open, creat, read, write, fstat, unlink.
// Supports LFN (long filename) entries.

extern crate alloc;

use alloc::vec::Vec;
use alloc::vec;
use alloc::sync::Arc;

// ---------------------------------------------------------------------------
// FAT32 on-disk structures (little-endian, packed)
// ---------------------------------------------------------------------------

/// BIOS Parameter Block — at offset 0 in the boot sector.
///
/// Must be `packed`: the FAT32 BPB has no alignment padding on disk.
/// Without `packed`, repr(C) inserts a byte after oem_name (offset 10→11)
/// to align bytes_per_sec to 2, which shifts root_clus from byte 44 to byte 48
/// and makes it read fs_info+bk_boot_sec (0x00060001) instead of the real value (2).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Fat32Bpb {
    jump_boot:       [u8; 3],
    oem_name:        [u8; 8],
    bytes_per_sec:   u16,
    sec_per_clus:    u8,
    rsvd_sec_cnt:    u16,
    num_fats:        u8,
    root_ent_cnt:    u16,
    tot_sec16:       u16,
    media:           u8,
    fat_sz16:        u16,
    sec_per_trk:     u16,
    num_heads:       u16,
    hidd_sec:        u32,
    tot_sec32:       u32,
    fat_sz32:        u32,
    ext_flags:       u16,
    fs_ver:          u16,
    root_clus:       u32,
    fs_info:         u16,
    bk_boot_sec:     u16,
    reserved:        [u8; 12],
    drv_num:         u8,
    reserved1:       u8,
    boot_sig:        u8,
    vol_id:          u32,
    vol_lab:         [u8; 11],
    fil_sys_type:    [u8; 8],
}

/// FAT32 directory entry — 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Fat32DirEntry {
    name:       [u8; 11],
    attr:       u8,
    nt_res:     u8,
    crt_time_tenth: u8,
    crt_time:   u16,
    crt_date:   u16,
    lst_acc_date: u16,
    fst_clus_hi: u16,
    wrt_time:   u16,
    wrt_date:   u16,
    fst_clus_lo: u16,
    file_size:  u32,
}

/// FAT32 LFN directory entry — 32 bytes.
///
/// Must be `packed`: name1 starts at disk byte 1 (odd offset), which
/// repr(C) would pad to 2, misaligning the entire entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Fat32LfnEntry {
    ord:        u8,
    name1:      [u16; 5],
    attr:       u8,
    entry_type: u8,
    chksum:     u8,
    name2:      [u16; 6],
    fst_clus_lo: u16,
    name3:      [u16; 2],
}

// FAT32 attribute bits
const ATTR_DIRECTORY:  u8 = 0x10;
const ATTR_LONG_NAME:  u8 = 0x0F;
const ATTR_LONG_NAME_MASK: u8 = 0x3F;

// FAT32 cluster constants
const CLUSTER_FREE: u32 = 0x00000000;
const CLUSTER_EOF_MIN: u32 = 0x0FFFFFF8;
const FIRST_DATA_CLUSTER: u32 = 2;

// Directory entry sentinels
const DIR_ENTRY_FREE:    u8 = 0x00;
const DIR_ENTRY_DELETED: u8 = 0xE5;

// LFN order flag
const LFN_LAST_ENTRY: u8 = 0x40;

#[inline]
fn is_eof(cluster: u32) -> bool {
    cluster >= CLUSTER_EOF_MIN
}

#[inline]
fn is_bad(cluster: u32) -> bool {
    cluster == 0x0FFFFFF7
}

#[inline]
fn entry_cluster(entry: &Fat32DirEntry) -> u32 {
    let lo = u16::from_le(entry.fst_clus_lo) as u32;
    let hi = u16::from_le(entry.fst_clus_hi) as u32;
    (hi << 16) | lo
}

#[inline]
fn is_lfn(entry: &Fat32DirEntry) -> bool {
    entry.attr & ATTR_LONG_NAME_MASK == ATTR_LONG_NAME
}

// ---------------------------------------------------------------------------
// Global filesystem state
// ---------------------------------------------------------------------------

struct Fat32State {
    /// LBA offset of the partition start on the physical disk.
    ///
    /// All FAT32 LBA values stored in BPB fields are relative to the partition
    /// start.  Every `read_sectors_into` / `write_sectors_from` call adds this
    /// offset before issuing the I/O request to the block device.
    partition_start_lba: u64,
    bytes_per_sector:   u32,
    sectors_per_cluster: u32,
    bytes_per_cluster:  u32,
    fat_start_lba:      u64,
    fat_sectors:        u32,
    num_fats:           u32,
    data_start_lba:     u64,
    root_cluster:       u32,
    total_clusters:     u32,
    fsinfo_sector:      u64,
    free_clusters:      u32,
    fat_bitmap:         Vec<u8>,   // 1 bit per cluster (bit = used)
    cluster_buf:        Vec<u8>,   // reusable scratch buffer
    /// Volume label from BPB bytes 43–53 (11 bytes, space-padded, no NUL).
    volume_label:       [u8; 11],
    ready:              bool,
}

impl Fat32State {
    fn new() -> Self {
        Self {
            partition_start_lba: 0,
            bytes_per_sector: 0,
            sectors_per_cluster: 0,
            bytes_per_cluster: 0,
            fat_start_lba: 0,
            fat_sectors: 0,
            num_fats: 0,
            data_start_lba: 0,
            root_cluster: 0,
            total_clusters: 0,
            fsinfo_sector: 0,
            free_clusters: 0,
            fat_bitmap: Vec::new(),
            cluster_buf: Vec::new(),
            volume_label: [b' '; 11],
            ready: false,
        }
    }
}

struct SyncCell<T>(core::cell::UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static STATE: SyncCell<Option<Fat32State>> = SyncCell(core::cell::UnsafeCell::new(None));

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut Fat32State) -> R,
    R: Default,
{
    let state_opt = unsafe { &mut *STATE.0.get() };
    match state_opt.as_mut() {
        Some(state) => f(state),
        None => R::default(), // FAT32 not initialized — return a safe default
    }
}

// ---------------------------------------------------------------------------
// Open file descriptor state
// ---------------------------------------------------------------------------

pub struct Fat32File {
    pub dir_cluster:     u32,
    pub dir_entry_index: u32,   // entry index within dir cluster
    pub first_cluster:   u32,
    pub file_size:       u32,
    pub position:        u64,
}

// ---------------------------------------------------------------------------
// Sector I/O helpers
// ---------------------------------------------------------------------------

/// Read sectors relative to the mounted partition start.
///
/// `lba` is the partition-relative LBA (as stored in FAT32 BPB fields).
/// The absolute disk LBA = `state.partition_start_lba + lba`.
fn read_sectors_into(lba: u64, count: u32, buf: &mut [u8]) -> bool {
    let offset = unsafe {
        (*STATE.0.get()).as_ref().map(|s| s.partition_start_lba).unwrap_or(0)
    };
    crate::hal::disk::read_sectors(offset + lba, count, buf)
}

/// Write sectors relative to the mounted partition start.
fn write_sectors_from(lba: u64, count: u32, buf: &[u8]) -> bool {
    let offset = unsafe {
        (*STATE.0.get()).as_ref().map(|s| s.partition_start_lba).unwrap_or(0)
    };
    crate::hal::disk::write_sectors(offset + lba, count, buf)
}

fn cluster_lba(state: &Fat32State, cluster: u32) -> u64 {
    state.data_start_lba + (cluster - FIRST_DATA_CLUSTER) as u64 * state.sectors_per_cluster as u64
}

fn read_cluster(state: &mut Fat32State, cluster: u32) -> bool {
    if cluster < FIRST_DATA_CLUSTER {
        return false;
    }
    let lba = cluster_lba(state, cluster);
    let sectors = state.sectors_per_cluster;
    let buf = &mut state.cluster_buf;
    read_sectors_into(lba, sectors, buf)
}

fn write_cluster(state: &mut Fat32State, cluster: u32) -> bool {
    if cluster < FIRST_DATA_CLUSTER {
        return false;
    }
    let lba = cluster_lba(state, cluster);
    let sectors = state.sectors_per_cluster;
    let buf = state.cluster_buf.clone(); // avoid borrow issue
    write_sectors_from(lba, sectors, &buf)
}

// ---------------------------------------------------------------------------
// FAT table I/O (cached single-sector)
// ---------------------------------------------------------------------------

// Thread-local FAT sector cache (non-reentrant; single-core kernel).
static FAT_CACHE_SECTOR: SyncCell<u64> = SyncCell(core::cell::UnsafeCell::new(u64::MAX));
static FAT_CACHE_BUF: SyncCell<[u8; 512]> = SyncCell(core::cell::UnsafeCell::new([0u8; 512]));

fn read_fat_entry(state: &Fat32State, cluster: u32) -> Option<u32> {
    let fat_offset = cluster as u64 * 4;
    let fat_sector = fat_offset / 512;
    let entry_offset = (fat_offset % 512) as usize;

    let cache_sector = unsafe { *FAT_CACHE_SECTOR.0.get() };
    if cache_sector != state.fat_start_lba + fat_sector {
        let buf = unsafe { &mut *FAT_CACHE_BUF.0.get() };
        if !read_sectors_into(state.fat_start_lba + fat_sector, 1, buf) {
            return None;
        }
        unsafe { *FAT_CACHE_SECTOR.0.get() = state.fat_start_lba + fat_sector; }
    }

    let buf = unsafe { &*FAT_CACHE_BUF.0.get() };
    let raw = u32::from_le_bytes([
        buf[entry_offset],
        buf[entry_offset + 1],
        buf[entry_offset + 2],
        buf[entry_offset + 3],
    ]);
    Some(raw & 0x0FFFFFFF)
}

fn write_fat_entry(state: &Fat32State, cluster: u32, value: u32) -> bool {
    let fat_offset = cluster as u64 * 4;
    let fat_sector = fat_offset / 512;
    let entry_offset = (fat_offset % 512) as usize;

    for fat in 0..state.num_fats {
        let sector = state.fat_start_lba + fat_sector + fat as u64 * state.fat_sectors as u64;
        let mut buf = [0u8; 512];
        if !read_sectors_into(sector, 1, &mut buf) {
            return false;
        }
        let existing = u32::from_le_bytes([buf[entry_offset], buf[entry_offset+1], buf[entry_offset+2], buf[entry_offset+3]]);
        let new_value = (value & 0x0FFFFFFF) | (existing & 0xF0000000);
        let bytes = new_value.to_le_bytes();
        buf[entry_offset..entry_offset+4].copy_from_slice(&bytes);
        if !write_sectors_from(sector, 1, &buf) {
            return false;
        }
    }

    // Invalidate cache if we touched the cached sector.
    let cache_sector = unsafe { *FAT_CACHE_SECTOR.0.get() };
    if cache_sector == state.fat_start_lba + fat_sector {
        unsafe { *FAT_CACHE_SECTOR.0.get() = u64::MAX; }
    }

    true
}

// ---------------------------------------------------------------------------
// FAT bitmap helpers
// ---------------------------------------------------------------------------

fn bitmap_set(state: &mut Fat32State, cluster: u32) {
    if cluster < FIRST_DATA_CLUSTER || cluster >= state.total_clusters + 2 {
        return;
    }
    let bit = (cluster - FIRST_DATA_CLUSTER) as usize;
    if bit / 8 < state.fat_bitmap.len() {
        state.fat_bitmap[bit / 8] |= 1 << (bit % 8);
    }
}

fn bitmap_clear(state: &mut Fat32State, cluster: u32) {
    if cluster < FIRST_DATA_CLUSTER || cluster >= state.total_clusters + 2 {
        return;
    }
    let bit = (cluster - FIRST_DATA_CLUSTER) as usize;
    if bit / 8 < state.fat_bitmap.len() {
        state.fat_bitmap[bit / 8] &= !(1 << (bit % 8));
    }
}

fn bitmap_find_free(state: &Fat32State) -> Option<u32> {
    for i in 0..state.total_clusters as usize {
        if state.fat_bitmap[i / 8] & (1 << (i % 8)) == 0 {
            return Some(FIRST_DATA_CLUSTER + i as u32);
        }
    }
    None
}

fn build_fat_bitmap(state: &mut Fat32State) -> bool {
    for i in 0..(state.total_clusters + 2) {
        let entry = match read_fat_entry(state, i) {
            Some(e) => e,
            None => return false,
        };
        if entry != CLUSTER_FREE {
            bitmap_set(state, i);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Cluster chain helpers
// ---------------------------------------------------------------------------

fn cluster_at_index(state: &Fat32State, first_cluster: u32, index: u32) -> Option<u32> {
    let mut cluster = first_cluster;
    for _ in 0..index {
        if is_eof(cluster) || is_bad(cluster) {
            return None;
        }
        cluster = read_fat_entry(state, cluster)?;
    }
    Some(cluster)
}

fn get_last_cluster(state: &Fat32State, first_cluster: u32) -> Option<u32> {
    if first_cluster < FIRST_DATA_CLUSTER {
        return None;
    }
    let mut cluster = first_cluster;
    loop {
        let next = read_fat_entry(state, cluster)?;
        if is_eof(next) || is_bad(next) || next < FIRST_DATA_CLUSTER {
            return Some(cluster);
        }
        cluster = next;
    }
}

fn alloc_cluster(state: &mut Fat32State, first_cluster: u32) -> Option<(u32, u32)> {
    let new_cluster = bitmap_find_free(state)?;

    bitmap_set(state, new_cluster);
    if !write_fat_entry(state, new_cluster, 0x0FFFFFFF) {
        bitmap_clear(state, new_cluster);
        return None;
    }

    let new_first = if first_cluster == 0 {
        new_cluster
    } else {
        let last = get_last_cluster(state, first_cluster)?;
        if !write_fat_entry(state, last, new_cluster) {
            bitmap_clear(state, new_cluster);
            return None;
        }
        first_cluster
    };

    if state.free_clusters > 0 {
        state.free_clusters -= 1;
    }

    Some((new_cluster, new_first))
}

fn free_chain(state: &mut Fat32State, mut cluster: u32) {
    while cluster >= FIRST_DATA_CLUSTER && !is_eof(cluster) && !is_bad(cluster) {
        let next = match read_fat_entry(state, cluster) {
            Some(n) => n,
            None => break,
        };
        bitmap_clear(state, cluster);
        write_fat_entry(state, cluster, CLUSTER_FREE);
        if state.free_clusters < 0xFFFFFFFF {
            state.free_clusters += 1;
        }
        cluster = next;
    }
    if cluster >= FIRST_DATA_CLUSTER {
        bitmap_clear(state, cluster);
        write_fat_entry(state, cluster, CLUSTER_FREE);
        if state.free_clusters < 0xFFFFFFFF {
            state.free_clusters += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Filename helpers
// ---------------------------------------------------------------------------

/// Convert a FAT32 8.3 short name to a display string.
///
/// `nt_res` is the `NTRes` field from the directory entry:
///   bit 3 — base name should be displayed lowercase
///   bit 4 — extension should be displayed lowercase
fn short_name_to_str(name: &[u8; 11], nt_res: u8) -> alloc::string::String {
    let base_lower = (nt_res & 0x08) != 0;
    let ext_lower  = (nt_res & 0x10) != 0;

    let mut s = alloc::string::String::new();
    for i in 0..8 {
        if name[i] == b' ' { break; }
        let c = name[i];
        let c = if base_lower && c >= b'A' && c <= b'Z' { c + 32 } else { c };
        s.push(c as char);
    }
    if name[8] != b' ' {
        s.push('.');
        for i in 8..11 {
            if name[i] == b' ' { break; }
            let c = name[i];
            let c = if ext_lower && c >= b'A' && c <= b'Z' { c + 32 } else { c };
            s.push(c as char);
        }
    }
    s
}

/// Convert a filename to FAT32 8.3 short name (stored uppercase).
///
/// Returns the `NTRes` byte that records the original case:
///   bit 3 — base name was originally lowercase
///   bit 4 — extension was originally lowercase
///
/// Callers must write the returned value into `Fat32DirEntry::nt_res` so
/// that `short_name_to_str` can reconstruct the original case on display.
fn name_to_short(name: &str, out: &mut [u8; 11]) -> u8 {
    out.fill(b' ');
    let dot_pos = name.find('.');
    let (base, ext) = match dot_pos {
        Some(pos) => (&name[..pos], &name[pos+1..]),
        None => (name, ""),
    };

    // NTRes bit 3 = "base is entirely lowercase" (no uppercase letters present).
    // NTRes bit 4 = "extension is entirely lowercase".
    // Mixed-case names (e.g. "Hola", "Holis.txt") cannot be represented via
    // NTRes alone — that requires LFN entries (not yet implemented).  For mixed
    // case we set neither bit and display the stored-uppercase form (e.g. "HOLA").
    let base_has_upper = base.bytes().any(|c| c >= b'A' && c <= b'Z');
    let base_has_lower = !base_has_upper && base.bytes().any(|c| c >= b'a' && c <= b'z');
    let ext_has_upper  = ext.bytes().any(|c| c >= b'A' && c <= b'Z');
    let ext_has_lower  = !ext_has_upper && ext.bytes().any(|c| c >= b'a' && c <= b'z');

    for (i, c) in base.bytes().enumerate().take(8) {
        out[i] = if c >= b'a' && c <= b'z' { c - 32 } else { c };
    }
    for (i, c) in ext.bytes().enumerate().take(3) {
        out[8 + i] = if c >= b'a' && c <= b'z' { c - 32 } else { c };
    }

    let mut nt_res: u8 = 0;
    if base_has_lower { nt_res |= 0x08; }
    if ext_has_lower  { nt_res |= 0x10; }
    nt_res
}

fn vfat_checksum(name: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for i in 0..11 {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(name[i]);
    }
    sum
}

fn names_match_case_insensitive(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    a.bytes().zip(b.bytes()).all(|(x, y)| {
        let x = if x >= b'A' && x <= b'Z' { x + 32 } else { x };
        let y = if y >= b'A' && y <= b'Z' { y + 32 } else { y };
        x == y
    })
}

fn lfn_char_to_ascii(ch: u16) -> char {
    // Preserve original case — LFN stores the exact name as entered.
    if ch < 0x80 { ch as u8 as char } else { '?' }
}

// ---------------------------------------------------------------------------
// Directory scanning
// ---------------------------------------------------------------------------

struct DirEntry {
    entry:             Fat32DirEntry,
    lfn_name:          alloc::string::String,
    entry_index:       u32,   // slot index of the 8.3 entry within its cluster
    cluster:           u32,   // cluster containing the 8.3 entry
    // LFN deletion tracking — zero when no LFN entries precede this 8.3 entry.
    lfn_count:         u32,   // number of LFN entries in the set
    lfn_start_cluster: u32,   // cluster containing the first (highest-ordinal) LFN entry
    lfn_start_index:   u32,   // slot index of that first LFN entry
}

fn lookup_in_dir(
    state: &mut Fat32State,
    dir_cluster: u32,
    component: &str,
) -> Option<DirEntry> {
    let entries_per_cluster = (state.bytes_per_cluster / 32) as u32;
    let mut cluster = dir_cluster;
    let mut lfn_buf = alloc::string::String::new();
    let mut lfn_expected_ord: i32 = 0;
    let mut lfn_start_cluster = 0u32;
    let mut lfn_start_index   = 0u32;
    let mut lfn_count         = 0u32;

    while !is_eof(cluster) && !is_bad(cluster) {
        // Read cluster into scratch buffer.
        if !read_cluster(state, cluster) {
            return None;
        }
        let cluster_data = state.cluster_buf.clone();
        let bytes_per_cluster = state.bytes_per_cluster as usize;

        let entry_count = bytes_per_cluster / 32;
        for i in 0..entry_count {
            let raw = &cluster_data[i * 32..(i + 1) * 32];
            let entry: Fat32DirEntry = unsafe { core::ptr::read_unaligned(raw.as_ptr() as *const Fat32DirEntry) };

            if entry.name[0] == DIR_ENTRY_FREE {
                return None;
            }
            if entry.name[0] == DIR_ENTRY_DELETED {
                lfn_buf.clear();
                lfn_expected_ord = 0;
                lfn_count = 0;
                lfn_start_cluster = 0;
                lfn_start_index   = 0;
                continue;
            }

            if is_lfn(&entry) {
                let lfn: Fat32LfnEntry = unsafe { core::ptr::read_unaligned(raw.as_ptr() as *const Fat32LfnEntry) };
                let ord = (lfn.ord & 0x1F) as i32;

                if lfn.ord & LFN_LAST_ENTRY != 0 {
                    lfn_expected_ord  = ord;
                    lfn_buf.clear();
                    lfn_start_cluster = cluster;
                    lfn_start_index   = i as u32;
                    lfn_count         = ord as u32;
                }

                if ord == lfn_expected_ord {
                    // Collect 13 UCS-2 characters.
                    // Copy packed arrays out before iterating — iterating over a
                    // reference to a packed field is UB; read_unaligned is required.
                    let name1: [u16; 5]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name1)) };
                    let name2: [u16; 6]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name2)) };
                    let name3: [u16; 2]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name3)) };
                    let mut chars = [0u16; 13];
                    for (j, c) in name1.iter().enumerate() { chars[j] = u16::from_le(*c); }
                    for (j, c) in name2.iter().enumerate() { chars[5 + j] = u16::from_le(*c); }
                    for (j, c) in name3.iter().enumerate() { chars[11 + j] = u16::from_le(*c); }

                    let mut segment = alloc::string::String::new();
                    for ch in chars.iter() {
                        if *ch == 0x0000 || *ch == 0xFFFF { break; }
                        segment.push(lfn_char_to_ascii(*ch));
                    }
                    // LFN entries arrive in reverse order — prepend.
                    let combined = segment + &lfn_buf;
                    lfn_buf = combined;
                    lfn_expected_ord -= 1;
                }
                continue;
            }

            // Regular entry: compare with component.
            let matched = if !lfn_buf.is_empty() {
                names_match_case_insensitive(&lfn_buf, component)
            } else {
                let short = short_name_to_str(&entry.name, entry.nt_res);
                names_match_case_insensitive(&short, component)
            };

            if matched {
                return Some(DirEntry {
                    entry,
                    lfn_name: lfn_buf,
                    entry_index:       i as u32,
                    cluster,
                    lfn_count,
                    lfn_start_cluster,
                    lfn_start_index,
                });
            }

            lfn_buf.clear();
            lfn_expected_ord  = 0;
            lfn_count         = 0;
            lfn_start_cluster = 0;
            lfn_start_index   = 0;
        }

        cluster = read_fat_entry(state, cluster)?;
    }

    None
}

// Resolve a path like "/hello.txt" or "/dir/file.txt" from the root cluster.
// Returns (dir_entry, parent_cluster) or None.
fn resolve_path(state: &mut Fat32State, path: &str) -> Option<(Fat32DirEntry, u32, u32)> {
    // Split path on '/'.
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }

    let root_cluster = state.root_cluster;
    let mut current_cluster = root_cluster;

    for (idx, part) in parts.iter().enumerate() {
        let found = lookup_in_dir(state, current_cluster, part)?;
        if idx == parts.len() - 1 {
            // Last component: return (entry, parent_cluster, entry_index).
            return Some((found.entry, current_cluster, found.entry_index));
        }
        // Intermediate component: must be a directory.
        if found.entry.attr & ATTR_DIRECTORY == 0 {
            return None;
        }
        current_cluster = entry_cluster(&found.entry);
    }
    None
}

// Strip //disk:, //disk:/mnt/ or /mnt/ prefix.
fn strip_disk_prefix(path: &str) -> &str {
    let path = if let Some(rest) = path.strip_prefix("//disk:") {
        rest
    } else {
        path
    };
    if let Some(rest) = path.strip_prefix("/mnt") {
        if rest.starts_with('/') || rest.is_empty() {
            return if rest.is_empty() { "/" } else { rest };
        }
    }
    path
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the FAT32 driver. Returns true on success.
/// Initialise the FAT32 driver from a specific partition on a `BlockDevice`.
///
/// `start_lba` is the first LBA of the partition as reported by the partition
/// table parser.  All subsequent sector reads and writes will be offset by
/// `start_lba` relative to the block device.
///
/// Returns `true` on success.  Replaces any previously initialised state.
///
/// # Panics / Safety
/// Caller must ensure IRQs are disabled (single-core invariant).
pub fn fat32_init_partition(disk: &dyn crate::hal::disk::BlockDevice, start_lba: u64) -> bool {
    // Prime the STATE with the partition offset so that read_sectors_into
    // applies the correct LBA translation when fat32_init() reads the BPB.
    {
        let state_opt = unsafe { &mut *STATE.0.get() };
        let state = state_opt.get_or_insert_with(Fat32State::new);
        state.partition_start_lba = start_lba;
        state.ready = false;
    }
    // Delegate to the existing BPB-parsing logic.
    // fat32_init() reads sector 0 (which becomes start_lba + 0 on disk).
    //
    // Note: fat32_init() also checks `hal::disk::capacity()` which reads the
    // global virtio device — this is fine for the current single-disk setup.
    // For multi-disk support, Fat32State would store the Arc<dyn BlockDevice>
    // and read_sectors_into would dispatch through it.
    let _ = disk; // reserved for future multi-disk dispatch
    fat32_init_inner()
}

/// `fat32_init` without partition offset — whole-disk mount (legacy / fallback).
pub fn fat32_init() -> bool {
    {
        let state_opt = unsafe { &mut *STATE.0.get() };
        let state = state_opt.get_or_insert_with(Fat32State::new);
        state.partition_start_lba = 0;
    }
    fat32_init_inner()
}

fn fat32_init_inner() -> bool {
    let state_opt = unsafe { &mut *STATE.0.get() };
    if state_opt.is_none() {
        *state_opt = Some(Fat32State::new());
    }
    let state = state_opt.as_mut().unwrap();

    let capacity = crate::hal::disk::capacity();
    if capacity == 0 {
        crate::drivers::uart::puts("[fat32] disk capacity is 0 — no disk\r\n");
        *state_opt = None;
        return false;
    }

    // Allocate sector buffer.
    state.cluster_buf = vec![0u8; 512];

    // Read boot sector (LBA 0 relative to partition start).
    if !read_sectors_into(0, 1, &mut state.cluster_buf) {
        crate::drivers::uart::puts("[fat32] failed to read boot sector\r\n");
        *state_opt = None;
        return false;
    }

    if state.cluster_buf[510] != 0x55 || state.cluster_buf[511] != 0xAA {
        crate::drivers::uart::puts("[fat32] bad boot signature (not 0x55AA)\r\n");
        *state_opt = None;
        return false;
    }

    // Check FAT32 signature.
    if &state.cluster_buf[82..87] != b"FAT32" {
        crate::drivers::uart::puts("[fat32] not a FAT32 volume (signature mismatch at offset 82)\r\n");
        *state_opt = None;
        return false;
    }

    let bpb: Fat32Bpb = unsafe { core::ptr::read_unaligned(state.cluster_buf.as_ptr() as *const Fat32Bpb) };

    state.bytes_per_sector   = u16::from_le(bpb.bytes_per_sec) as u32;
    state.sectors_per_cluster = bpb.sec_per_clus as u32;
    state.bytes_per_cluster  = state.bytes_per_sector * state.sectors_per_cluster;
    state.fat_start_lba      = u16::from_le(bpb.rsvd_sec_cnt) as u64;
    state.fat_sectors        = u32::from_le(bpb.fat_sz32);
    state.num_fats           = bpb.num_fats as u32;
    state.data_start_lba     = state.fat_start_lba + state.num_fats as u64 * state.fat_sectors as u64;
    state.root_cluster       = u32::from_le(bpb.root_clus);
    state.fsinfo_sector      = u16::from_le(bpb.fs_info) as u64;
    state.volume_label       = bpb.vol_lab;

    let total_sectors = if bpb.tot_sec16 != 0 {
        u16::from_le(bpb.tot_sec16) as u64
    } else {
        u32::from_le(bpb.tot_sec32) as u64
    };
    let data_sectors = total_sectors.saturating_sub(state.data_start_lba);
    state.total_clusters = (data_sectors / state.sectors_per_cluster as u64) as u32;

    // Resize cluster_buf to full cluster size.
    state.cluster_buf = vec![0u8; state.bytes_per_cluster as usize];

    // Allocate and build FAT bitmap.
    let bitmap_size = ((state.total_clusters + 7) / 8) as usize;
    state.fat_bitmap = vec![0u8; bitmap_size];

    if !build_fat_bitmap(state) {
        crate::drivers::uart::puts("[fat32] failed to build FAT bitmap\r\n");
        *state_opt = None;
        return false;
    }

    // Read FSInfo for free cluster count.
    let mut fsinfo_buf = [0u8; 512];
    if read_sectors_into(state.fsinfo_sector, 1, &mut fsinfo_buf) {
        let sig1 = u32::from_le_bytes([fsinfo_buf[0], fsinfo_buf[1], fsinfo_buf[2], fsinfo_buf[3]]);
        let sig2 = u32::from_le_bytes([fsinfo_buf[484], fsinfo_buf[485], fsinfo_buf[486], fsinfo_buf[487]]);
        if sig1 == 0x41615252 && sig2 == 0x61417272 {
            state.free_clusters = u32::from_le_bytes([fsinfo_buf[488], fsinfo_buf[489], fsinfo_buf[490], fsinfo_buf[491]]);
        } else {
            state.free_clusters = state.total_clusters;
        }
    } else {
        state.free_clusters = state.total_clusters;
    }

    state.ready = true;
    crate::drivers::uart::puts("[fat32] initialized — ");
    crate::drivers::uart::put_hex(state.total_clusters as u64);
    crate::drivers::uart::puts(" clusters, root @ cluster ");
    crate::drivers::uart::put_hex(state.root_cluster as u64);
    crate::drivers::uart::puts("\r\n");
    true
}

/// Open a file for reading. Returns None if not found.
pub fn fat32_open(path: &str) -> Option<Fat32File> {
    let path = strip_disk_prefix(path);
    with_state(|state| {
        if !state.ready { return None; }
        let (entry, parent_cluster, entry_index) = resolve_path(state, path)?;
        if entry.attr & ATTR_DIRECTORY != 0 { return None; }
        Some(Fat32File {
            dir_cluster:     parent_cluster,
            dir_entry_index: entry_index,
            first_cluster:   entry_cluster(&entry),
            file_size:       u32::from_le(entry.file_size),
            position:        0,
        })
    })
}

/// Read up to `buf.len()` bytes from `file` at its current position.
/// Returns bytes read, or -1 on error.
pub fn fat32_read(file: &mut Fat32File, buf: &mut [u8]) -> i64 {
    with_state(|state| {
        if !state.ready { return -1; }

        let file_size = file.file_size as u64;
        if file.position >= file_size {
            return 0;
        }

        let available = file_size - file.position;
        let to_read = (buf.len() as u64).min(available) as usize;
        let mut total_read = 0usize;
        let mut pos = file.position;

        while total_read < to_read {
            let cluster_index = (pos / state.bytes_per_cluster as u64) as u32;
            let cluster = match cluster_at_index(state, file.first_cluster, cluster_index) {
                Some(c) if c >= FIRST_DATA_CLUSTER => c,
                _ => break,
            };

            if !read_cluster(state, cluster) {
                break;
            }

            let offset_in_cluster = (pos % state.bytes_per_cluster as u64) as usize;
            let available_in_cluster = state.bytes_per_cluster as usize - offset_in_cluster;
            let chunk = (to_read - total_read).min(available_in_cluster);

            let cluster_data = state.cluster_buf.clone();
            buf[total_read..total_read + chunk]
                .copy_from_slice(&cluster_data[offset_in_cluster..offset_in_cluster + chunk]);

            total_read += chunk;
            pos += chunk as u64;
        }

        file.position = pos;
        total_read as i64
    })
}

/// Write `buf` to `file` at its current position (extending if necessary).
/// Returns bytes written, or -1 on error.
pub fn fat32_write(file: &mut Fat32File, buf: &[u8]) -> i64 {
    if buf.is_empty() { return 0; }

    let state_opt = unsafe { &mut *STATE.0.get() };
    let state = match state_opt.as_mut() {
        Some(s) if s.ready => s,
        _ => return -1,
    };

    let bytes_per_cluster = state.bytes_per_cluster as u64;
    let mut total_written = 0usize;
    let mut pos = file.position;
    let to_write = buf.len();

    while total_written < to_write {
        let cluster_index = (pos / bytes_per_cluster) as u32;

        // Get or allocate cluster.
        let cluster = if file.first_cluster < FIRST_DATA_CLUSTER {
            match alloc_cluster(state, 0) {
                Some((new_cluster, new_first)) => { file.first_cluster = new_first; new_cluster }
                None => break,
            }
        } else {
            match cluster_at_index(state, file.first_cluster, cluster_index) {
                Some(c) if c >= FIRST_DATA_CLUSTER => c,
                _ => {
                    match alloc_cluster(state, file.first_cluster) {
                        Some((new_cluster, new_first)) => { file.first_cluster = new_first; new_cluster }
                        None => break,
                    }
                }
            }
        };

        // Read existing cluster data (for partial writes).
        let offset_in_cluster = (pos % bytes_per_cluster) as usize;
        read_cluster(state, cluster); // ignore error — partial init OK

        let available_in_cluster = state.bytes_per_cluster as usize - offset_in_cluster;
        let chunk = (to_write - total_written).min(available_in_cluster);

        // Write chunk into cluster buffer.
        state.cluster_buf[offset_in_cluster..offset_in_cluster + chunk]
            .copy_from_slice(&buf[total_written..total_written + chunk]);

        if !write_cluster(state, cluster) {
            break;
        }

        total_written += chunk;
        pos += chunk as u64;
    }

    file.position = pos;
    if pos > file.file_size as u64 {
        file.file_size = pos as u32;
    }

    // Update directory entry size.
    update_dir_entry_size(state, file);

    total_written as i64
}

fn update_dir_entry_size(state: &mut Fat32State, file: &Fat32File) {
    // Read the directory cluster containing the file's entry.
    if !read_cluster(state, file.dir_cluster) {
        return;
    }
    let idx = file.dir_entry_index as usize;
    let entry_offset = idx * 32;
    if entry_offset + 32 > state.cluster_buf.len() {
        return;
    }
    // Patch file_size in cluster buffer.
    let size_bytes = file.file_size.to_le_bytes();
    let offset = entry_offset + 28; // file_size is at offset 28 within 32-byte entry
    state.cluster_buf[offset..offset + 4].copy_from_slice(&size_bytes);

    // Also patch first cluster fields.
    let lo = (file.first_cluster & 0xFFFF) as u16;
    let hi = (file.first_cluster >> 16) as u16;
    let lo_bytes = lo.to_le_bytes();
    let hi_bytes = hi.to_le_bytes();
    // fst_clus_lo at offset 26, fst_clus_hi at offset 20 within 32-byte entry.
    state.cluster_buf[entry_offset + 26..entry_offset + 28].copy_from_slice(&lo_bytes);
    state.cluster_buf[entry_offset + 20..entry_offset + 22].copy_from_slice(&hi_bytes);

    write_cluster(state, file.dir_cluster);
}

/// Create a new empty file. Returns None on failure.
/// Create or open a file for writing.
///
/// `truncate = true`  → O_WRONLY|O_CREAT|O_TRUNC semantics (used by `>`).
/// `truncate = false` → O_WRONLY|O_CREAT semantics, no truncation (used by `>>`).
pub fn fat32_creat(path: &str, truncate: bool) -> Option<Fat32File> {
    let path = strip_disk_prefix(path);
    with_state(|state| {
        if !state.ready { return None; }

        // Split to get parent dir and filename.
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() { return None; }

        let filename = parts.last().copied()?;
        let dir_cluster = if parts.len() == 1 {
            state.root_cluster
        } else {
            // Navigate to parent directory.
            let mut current = state.root_cluster;
            for part in &parts[..parts.len()-1] {
                let found = lookup_in_dir(state, current, part)?;
                if found.entry.attr & ATTR_DIRECTORY == 0 { return None; }
                current = entry_cluster(&found.entry);
            }
            current
        };

        // File already exists.
        if let Some(dir_entry) = lookup_in_dir(state, dir_cluster, filename) {
            if truncate {
                // O_TRUNC: free old cluster chain, reset size to 0.
                let first_cluster = entry_cluster(&dir_entry.entry);
                if first_cluster >= FIRST_DATA_CLUSTER {
                    free_chain(state, first_cluster);
                }
                let mut truncated = Fat32File {
                    dir_cluster,
                    dir_entry_index: dir_entry.entry_index,
                    first_cluster: 0,
                    file_size: 0,
                    position: 0,
                };
                update_dir_entry_size(state, &truncated);
                return Some(truncated);
            } else {
                // No truncation: open the existing file as-is (append mode).
                return Some(Fat32File {
                    dir_cluster,
                    dir_entry_index: dir_entry.entry_index,
                    first_cluster: entry_cluster(&dir_entry.entry),
                    file_size: u32::from_le(dir_entry.entry.file_size),
                    position: 0,
                });
            }
        }

        // Write directory entry — with LFN if the name requires it.
        let (entry_cluster_num, entry_index) = if needs_lfn(filename) {
            let short_name = generate_short_alias(state, dir_cluster, filename)?;
            let n_slots    = lfn_entry_count(filename) + 1;
            let (sc, si)   = find_n_free_dir_slots(state, dir_cluster, n_slots)?;
            write_lfn_and_83(state, sc, si, filename, &short_name, 0x20, 0, 0)?
        } else {
            let mut short_name = [b' '; 11];
            let nt_res = name_to_short(filename, &mut short_name);
            let (sc, si) = find_free_dir_slot(state, dir_cluster)?;
            let mut new_entry = Fat32DirEntry::default();
            new_entry.name   = short_name;
            new_entry.nt_res = nt_res;
            new_entry.attr   = 0x20; // ARCHIVE
            if !read_cluster(state, sc) { return None; }
            let entry_bytes = unsafe {
                core::slice::from_raw_parts(&new_entry as *const Fat32DirEntry as *const u8, 32)
            };
            state.cluster_buf[si as usize * 32..si as usize * 32 + 32].copy_from_slice(entry_bytes);
            if !write_cluster(state, sc) { return None; }
            (sc, si)
        };

        Some(Fat32File {
            dir_cluster:     entry_cluster_num,
            dir_entry_index: entry_index,
            first_cluster:   0,
            file_size:       0,
            position:        0,
        })
    })
}

fn find_free_dir_slot(state: &mut Fat32State, dir_cluster: u32) -> Option<(u32, u32)> {
    let entries_per_cluster = (state.bytes_per_cluster / 32) as usize;
    let mut cluster = dir_cluster;

    while !is_eof(cluster) && !is_bad(cluster) {
        if !read_cluster(state, cluster) { return None; }
        let cluster_data = state.cluster_buf.clone();

        for i in 0..entries_per_cluster {
            let first_byte = cluster_data[i * 32];
            if first_byte == DIR_ENTRY_FREE || first_byte == DIR_ENTRY_DELETED {
                return Some((cluster, i as u32));
            }
        }

        let next = read_fat_entry(state, cluster)?;
        cluster = next;
    }

    // Need to extend the directory.
    let (new_cluster, _) = alloc_cluster(state, dir_cluster)?;
    // Zero the new cluster.
    state.cluster_buf.fill(0);
    write_cluster(state, new_cluster);
    Some((new_cluster, 0))
}

// ---------------------------------------------------------------------------
// LFN helpers — Long File Name support (VFAT extension, FAT32 spec §7)
// ---------------------------------------------------------------------------

/// Returns true when the name requires LFN directory entries.
///
/// LFN is needed when any of these hold:
///   - base component > 8 chars or extension > 3 chars
///   - mixed case within base or extension ("Hola", "ReadMe.Txt")
///   - contains characters illegal in 8.3 names (space, +, ,, ;, =, [, ])
fn needs_lfn(name: &str) -> bool {
    let dot_pos = name.rfind('.');
    let (base, ext) = match dot_pos {
        Some(pos) => (&name[..pos], &name[pos+1..]),
        None => (name, ""),
    };
    if base.len() > 8 || ext.len() > 3 { return true; }
    for b in name.bytes() {
        if b" +,;=[]".contains(&b) { return true; }
    }
    let base_up = base.bytes().any(|c| c >= b'A' && c <= b'Z');
    let base_lo = base.bytes().any(|c| c >= b'a' && c <= b'z');
    if base_up && base_lo { return true; }
    let ext_up = ext.bytes().any(|c| c >= b'A' && c <= b'Z');
    let ext_lo = ext.bytes().any(|c| c >= b'a' && c <= b'z');
    ext_up && ext_lo
}

/// Number of LFN directory entries needed to store `name` (ceil(len / 13)).
fn lfn_entry_count(name: &str) -> usize {
    (name.len() + 12) / 13
}

/// Generate a unique 8.3 alias for a long name (e.g. "MiCarpeta" → "MICAR~1   ").
///
/// Returns `None` if no alias can be found (directory has >9999 similarly
/// named entries — effectively impossible in practice).
fn generate_short_alias(
    state:       &mut Fat32State,
    dir_cluster: u32,
    name:        &str,
) -> Option<[u8; 11]> {
    let dot_pos = name.rfind('.');
    let (base_part, ext_part) = match dot_pos {
        Some(pos) => (&name[..pos], &name[pos+1..]),
        None => (name, ""),
    };

    // Collect up to 6 valid 8.3 base chars (uppercase, skip illegal chars).
    let mut base_chars = [b' '; 6];
    let mut base_len = 0usize;
    for c in base_part.bytes() {
        if base_len >= 6 { break; }
        if b" +,;=[]".contains(&c) || c == b'.' { continue; }
        base_chars[base_len] = if c >= b'a' && c <= b'z' { c - 32 } else { c };
        base_len += 1;
    }

    // Extension: up to 3 chars, uppercased.
    let mut ext_bytes = [b' '; 3];
    for (i, c) in ext_part.bytes().enumerate().take(3) {
        ext_bytes[i] = if c >= b'a' && c <= b'z' { c - 32 } else { c };
    }

    for n in 1u32..=9999 {
        let suffix = alloc::format!("~{}", n);
        let suffix_bytes = suffix.as_bytes();
        let base_use = base_len.min(8 - suffix_bytes.len());

        let mut short = [b' '; 11];
        short[..base_use].copy_from_slice(&base_chars[..base_use]);
        for (i, &c) in suffix_bytes.iter().enumerate() {
            short[base_use + i] = c;
        }
        short[8]  = ext_bytes[0];
        short[9]  = ext_bytes[1];
        short[10] = ext_bytes[2];

        // Accept if no existing entry has this alias.
        let alias_str = short_name_to_str(&short, 0);
        if lookup_in_dir(state, dir_cluster, &alias_str).is_none() {
            return Some(short);
        }
    }
    None
}

/// Find `count` consecutive free / deleted directory slots in the chain rooted
/// at `dir_cluster`.  Extends the directory by allocating new FAT clusters when
/// no suitable run exists.
///
/// Returns `(cluster, first_index)` of the run start, or `None` on failure.
fn find_n_free_dir_slots(
    state:       &mut Fat32State,
    dir_cluster: u32,
    count:       usize,
) -> Option<(u32, u32)> {
    if count <= 1 {
        return find_free_dir_slot(state, dir_cluster);
    }

    let entries_per_cluster = (state.bytes_per_cluster / 32) as usize;
    let mut cluster = dir_cluster;

    while !is_eof(cluster) && !is_bad(cluster) {
        if !read_cluster(state, cluster) { return None; }
        let cluster_data = state.cluster_buf.clone();

        let mut run_start: Option<u32> = None;
        let mut run_len = 0usize;

        for i in 0..entries_per_cluster {
            let fb = cluster_data[i * 32];
            if fb == DIR_ENTRY_FREE || fb == DIR_ENTRY_DELETED {
                if run_len == 0 { run_start = Some(i as u32); }
                run_len += 1;
                if run_len >= count {
                    return Some((cluster, run_start.unwrap()));
                }
                // 0x00 means all following entries in this cluster are also free.
                if fb == DIR_ENTRY_FREE {
                    let free_from_here = entries_per_cluster - run_start.unwrap() as usize;
                    if free_from_here >= count {
                        return Some((cluster, run_start.unwrap()));
                    }
                    break; // not enough in this cluster alone — extend below
                }
            } else {
                run_len  = 0;
                run_start = None;
            }
        }

        match read_fat_entry(state, cluster) {
            Some(n) if !is_eof(n) && !is_bad(n) => cluster = n,
            _ => break,
        }
    }

    // No suitable run found — append new cluster(s) at the end of the directory.
    let clusters_needed = (count + entries_per_cluster - 1) / entries_per_cluster;
    let mut first_new = 0u32;
    for k in 0..clusters_needed {
        let (new_c, _) = alloc_cluster(state, dir_cluster)?;
        if k == 0 { first_new = new_c; }
        state.cluster_buf.fill(0);
        write_cluster(state, new_c);
    }
    Some((first_new, 0))
}

/// Write LFN entries (highest ordinal first) followed by the 8.3 directory
/// entry, starting at `(start_cluster, start_index)`.  Crosses cluster
/// boundaries by following the FAT chain.
///
/// Returns `(cluster, index)` of the written 8.3 entry, or `None` on error.
fn write_lfn_and_83(
    state:         &mut Fat32State,
    start_cluster: u32,
    start_index:   u32,
    name:          &str,
    short_name:    &[u8; 11],
    attr:          u8,
    file_cluster:  u32,
    file_size:     u32,
) -> Option<(u32, u32)> {
    let checksum  = vfat_checksum(short_name);
    let n_lfn     = lfn_entry_count(name);

    // Encode name as UCS-2 LE (ASCII range only; non-ASCII mapped to '?').
    let name_ucs2: Vec<u16> = name.chars()
        .map(|c| if (c as u32) < 0x80 { c as u16 } else { b'?' as u16 })
        .collect();

    let entries_per_cluster = (state.bytes_per_cluster / 32) as usize;
    let mut cur_cluster = start_cluster;
    let mut cur_index   = start_index as usize;

    // Load the first cluster to preserve any existing entries around our slots.
    if !read_cluster(state, cur_cluster) { return None; }

    // Write LFN entries from highest ordinal (first in directory, last chars)
    // down to ordinal 1 (last before 8.3, first chars).
    for lfn_num in (1..=(n_lfn as u8)).rev() {
        let seg_start = (lfn_num as usize - 1) * 13;
        let mut chars = [0xFFFFu16; 13];
        for j in 0..13 {
            let idx = seg_start + j;
            if idx < name_ucs2.len() {
                chars[j] = name_ucs2[idx];
            } else if idx == name_ucs2.len() {
                chars[j] = 0x0000; // null terminator
            }
            // else: 0xFFFF pad already set
        }

        // Highest ordinal gets the LAST_LONG_ENTRY flag.
        let ord = if lfn_num == n_lfn as u8 { lfn_num | LFN_LAST_ENTRY } else { lfn_num };

        let mut lfn_bytes = [0u8; 32];
        lfn_bytes[0]  = ord;
        for j in 0..5usize {
            let le = chars[j].to_le_bytes();
            lfn_bytes[1 + j * 2] = le[0];
            lfn_bytes[2 + j * 2] = le[1];
        }
        lfn_bytes[11] = ATTR_LONG_NAME;
        lfn_bytes[12] = 0; // entry_type
        lfn_bytes[13] = checksum;
        for j in 0..6usize {
            let le = chars[5 + j].to_le_bytes();
            lfn_bytes[14 + j * 2] = le[0];
            lfn_bytes[15 + j * 2] = le[1];
        }
        lfn_bytes[26] = 0; // fst_clus_lo (must be 0)
        lfn_bytes[27] = 0;
        for j in 0..2usize {
            let le = chars[11 + j].to_le_bytes();
            lfn_bytes[28 + j * 2] = le[0];
            lfn_bytes[29 + j * 2] = le[1];
        }

        // Flush current cluster and advance when crossing a cluster boundary.
        if cur_index >= entries_per_cluster {
            if !write_cluster(state, cur_cluster) { return None; }
            let next = read_fat_entry(state, cur_cluster)?;
            if is_eof(next) || is_bad(next) { return None; }
            cur_cluster = next;
            cur_index   = 0;
            if !read_cluster(state, cur_cluster) { return None; }
        }

        let off = cur_index * 32;
        state.cluster_buf[off..off + 32].copy_from_slice(&lfn_bytes);
        cur_index += 1;
    }

    // Flush and advance before the 8.3 entry if at a cluster boundary.
    if cur_index >= entries_per_cluster {
        if !write_cluster(state, cur_cluster) { return None; }
        let next = read_fat_entry(state, cur_cluster)?;
        if is_eof(next) || is_bad(next) { return None; }
        cur_cluster = next;
        cur_index   = 0;
        if !read_cluster(state, cur_cluster) { return None; }
    }

    // Write the 8.3 entry.
    let mut entry_83 = Fat32DirEntry::default();
    entry_83.name        = *short_name;
    entry_83.attr        = attr;
    entry_83.fst_clus_lo = (file_cluster & 0xFFFF) as u16;
    entry_83.fst_clus_hi = ((file_cluster >> 16) & 0xFFFF) as u16;
    entry_83.file_size   = file_size.to_le();

    let entry_bytes = unsafe {
        core::slice::from_raw_parts(&entry_83 as *const Fat32DirEntry as *const u8, 32)
    };
    let off = cur_index * 32;
    state.cluster_buf[off..off + 32].copy_from_slice(entry_bytes);
    if !write_cluster(state, cur_cluster) { return None; }

    Some((cur_cluster, cur_index as u32))
}

/// Mark `count` consecutive directory entry slots as deleted (0xE5), starting
/// at `(start_cluster, start_index)`.  Follows the FAT chain across cluster
/// boundaries.  Used to remove the LFN entries that precede a deleted 8.3 entry.
fn mark_entries_deleted(
    state:         &mut Fat32State,
    start_cluster: u32,
    start_index:   u32,
    count:         u32,
) {
    if count == 0 { return; }
    let entries_per_cluster = (state.bytes_per_cluster / 32) as usize;
    let mut cluster   = start_cluster;
    let mut index     = start_index as usize;
    let mut remaining = count as usize;

    while remaining > 0 {
        if !read_cluster(state, cluster) { return; }
        let mut dirty = false;
        while index < entries_per_cluster && remaining > 0 {
            state.cluster_buf[index * 32] = DIR_ENTRY_DELETED;
            index     += 1;
            remaining -= 1;
            dirty      = true;
        }
        if dirty { write_cluster(state, cluster); }
        if remaining > 0 {
            match read_fat_entry(state, cluster) {
                Some(n) if !is_eof(n) && !is_bad(n) => { cluster = n; index = 0; }
                _ => break,
            }
        }
    }
}

/// Return (file_size, file_type) for a path. Returns None if not found.
pub fn fat32_fstat(path: &str) -> Option<(u64, i32)> {
    let path = strip_disk_prefix(path);
    with_state(|state| {
        if !state.ready { return None; }
        let (entry, _, _) = resolve_path(state, path)?;
        let size = u32::from_le(entry.file_size) as u64;
        let file_type = if entry.attr & ATTR_DIRECTORY != 0 { 2 } else { 1 };
        Some((size, file_type))
    })
}

/// List all entries in a FAT32 directory cluster chain.
///
/// Returns a Vec of `(name, is_directory, file_size, first_cluster)` tuples.
/// Used by `Fat32DirInode::readdir` and `Fat32DirInode::lookup`.
fn list_dir(state: &mut Fat32State, dir_cluster: u32) -> Vec<(alloc::string::String, bool, u32, u32)> {
    let entries_per_cluster = (state.bytes_per_cluster / 32) as usize;
    let mut cluster = dir_cluster;
    let mut results: Vec<(alloc::string::String, bool, u32, u32)> = Vec::new();
    let mut lfn_buf = alloc::string::String::new();
    let mut lfn_expected_ord: i32 = 0;

    while !is_eof(cluster) && !is_bad(cluster) {
        if !read_cluster(state, cluster) {
            break;
        }
        let cluster_data = state.cluster_buf.clone();

        for i in 0..entries_per_cluster {
            let raw = &cluster_data[i * 32..(i + 1) * 32];
            let entry: Fat32DirEntry = unsafe {
                core::ptr::read_unaligned(raw.as_ptr() as *const Fat32DirEntry)
            };

            if entry.name[0] == DIR_ENTRY_FREE {
                return results;
            }
            if entry.name[0] == DIR_ENTRY_DELETED {
                lfn_buf.clear();
                lfn_expected_ord = 0;
                continue;
            }

            // Skip volume label entries (ATTR_VOLUME_ID = 0x08, not a file or directory).
            if entry.attr == 0x08 {
                lfn_buf.clear();
                lfn_expected_ord = 0;
                continue;
            }

            if is_lfn(&entry) {
                let lfn: Fat32LfnEntry = unsafe {
                    core::ptr::read_unaligned(raw.as_ptr() as *const Fat32LfnEntry)
                };
                let ord = (lfn.ord & 0x1F) as i32;
                if lfn.ord & LFN_LAST_ENTRY != 0 {
                    lfn_expected_ord = ord;
                    lfn_buf.clear();
                }
                if ord == lfn_expected_ord {
                    let name1: [u16; 5]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name1)) };
                    let name2: [u16; 6]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name2)) };
                    let name3: [u16; 2]  = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(lfn.name3)) };
                    let mut chars = [0u16; 13];
                    for (j, c) in name1.iter().enumerate() { chars[j] = u16::from_le(*c); }
                    for (j, c) in name2.iter().enumerate() { chars[5 + j] = u16::from_le(*c); }
                    for (j, c) in name3.iter().enumerate() { chars[11 + j] = u16::from_le(*c); }
                    let mut segment = alloc::string::String::new();
                    for ch in chars.iter() {
                        if *ch == 0x0000 || *ch == 0xFFFF { break; }
                        segment.push(lfn_char_to_ascii(*ch));
                    }
                    let combined = segment + &lfn_buf;
                    lfn_buf = combined;
                    lfn_expected_ord -= 1;
                }
                continue;
            }

            // Skip "." and ".."
            let dot = entry.name[0] == b'.' && entry.name[1..].iter().all(|&b| b == b' ' || b == b'.');
            if !dot {
                let name = if !lfn_buf.is_empty() {
                    lfn_buf.clone()
                } else {
                    short_name_to_str(&entry.name, entry.nt_res)
                };
                let is_dir = entry.attr & ATTR_DIRECTORY != 0;
                let file_size = u32::from_le(entry.file_size);
                let first_cluster = entry_cluster(&entry);
                results.push((name, is_dir, file_size, first_cluster));
            }

            lfn_buf.clear();
            lfn_expected_ord = 0;
        }

        cluster = match read_fat_entry(state, cluster) {
            Some(c) => c,
            None => break,
        };
    }
    results
}

/// Delete a file. Returns true on success.
pub fn fat32_unlink(path: &str) -> bool {
    let path = strip_disk_prefix(path);
    let state_opt = unsafe { &mut *STATE.0.get() };
    let state = match state_opt.as_mut() {
        Some(s) if s.ready => s,
        _ => return false,
    };

    // Resolve path to parent dir cluster + filename.
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return false; }
    let filename = match parts.last() { Some(s) => *s, None => return false };
    let mut parent_cluster = state.root_cluster;
    for part in &parts[..parts.len()-1] {
        match lookup_in_dir(state, parent_cluster, part) {
            Some(d) if d.entry.attr & ATTR_DIRECTORY != 0 => {
                parent_cluster = entry_cluster(&d.entry);
            }
            _ => return false,
        }
    }

    let found = match lookup_in_dir(state, parent_cluster, filename) {
        Some(d) => d,
        None => return false,
    };

    if found.entry.attr & ATTR_DIRECTORY != 0 { return false; }

    // Free cluster chain.
    let first_cluster = entry_cluster(&found.entry);
    if first_cluster >= FIRST_DATA_CLUSTER {
        free_chain(state, first_cluster);
    }

    // Delete LFN entries.
    if found.lfn_count > 0 {
        mark_entries_deleted(state, found.lfn_start_cluster, found.lfn_start_index,
                             found.lfn_count);
    }

    // Mark 8.3 entry as deleted.
    if !read_cluster(state, found.cluster) { return false; }
    let offset = found.entry_index as usize * 32;
    if offset < state.cluster_buf.len() {
        state.cluster_buf[offset] = DIR_ENTRY_DELETED;
    }
    write_cluster(state, found.cluster);
    true
}

// ---------------------------------------------------------------------------
// VFS Inode wrappers — expose FAT32 through the kernel Inode trait so the
// VFS mount table can route /mnt/* paths to the FAT32 driver transparently.
//
// Design:
//   Fat32DirInode  — represents a FAT32 directory (holds its first cluster).
//   Fat32FileInode — represents a FAT32 regular file (holds metadata).
//
// Both types use the global FAT32_STATE via `with_state()`.  They assert
// Send+Sync because the kernel runs single-core with IRQs disabled.
// ---------------------------------------------------------------------------

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use super::inode::{
    Inode, InodeType, InodeStat, DirEntry as VfsDirEntry, FsError,
    alloc_inode_number,
};

// ---------------------------------------------------------------------------
// Fat32DirInode
// ---------------------------------------------------------------------------

/// VFS inode for a FAT32 directory.
pub struct Fat32DirInode {
    inode_number: u64,
    /// First cluster of this directory (root_cluster for "/").
    dir_cluster: u32,
}

unsafe impl Send for Fat32DirInode {}
unsafe impl Sync for Fat32DirInode {}

impl Fat32DirInode {
    fn new(dir_cluster: u32) -> Arc<dyn Inode> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            dir_cluster,
        })
    }
}

impl Inode for Fat32DirInode {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn stat(&self) -> InodeStat {
        InodeStat::directory(self.inode_number)
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let dir_cluster = self.dir_cluster;
        with_state(|state| {
            if !state.ready { return None; }
            let found = lookup_in_dir(state, dir_cluster, name)?;
            if found.entry.attr & ATTR_DIRECTORY != 0 {
                Some(Fat32DirInode::new(entry_cluster(&found.entry)))
            } else {
                Some(Fat32FileInode::new(
                    entry_cluster(&found.entry),
                    u32::from_le(found.entry.file_size),
                    dir_cluster,
                    found.entry_index,
                ))
            }
        })
    }

    fn readdir(&self, index: usize) -> Option<VfsDirEntry> {
        let dir_cluster = self.dir_cluster;
        with_state(|state| {
            if !state.ready { return None; }
            let entries = list_dir(state, dir_cluster);
            let (name, is_dir, _size, _cluster) = entries.into_iter().nth(index)?;
            Some(VfsDirEntry {
                name,
                inode_type: if is_dir { InodeType::Directory } else { InodeType::RegularFile },
                inode_number: alloc_inode_number(),
            })
        })
    }

    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let dir_cluster = self.dir_cluster;
        let name_owned = alloc::string::String::from(name);
        let state_opt = unsafe { &mut *STATE.0.get() };
        let state = match state_opt.as_mut() {
            Some(s) if s.ready => s,
            _ => return Err(FsError::IoError),
        };
        {

            // If the file already exists, return it.
            if let Some(existing) = lookup_in_dir(state, dir_cluster, &name_owned) {
                if existing.entry.attr & ATTR_DIRECTORY != 0 {
                    return Err(FsError::NotSupported);
                }
                return Ok(Fat32FileInode::new(
                    entry_cluster(&existing.entry),
                    u32::from_le(existing.entry.file_size),
                    existing.cluster,
                    existing.entry_index,
                ));
            }

            // Write directory entry — with LFN if the name requires it.
            let (entry_cluster_num, entry_index) = if needs_lfn(&name_owned) {
                let short_name = generate_short_alias(state, dir_cluster, &name_owned)
                    .ok_or(FsError::OutOfMemory)?;
                let n_slots  = lfn_entry_count(&name_owned) + 1;
                let (sc, si) = find_n_free_dir_slots(state, dir_cluster, n_slots)
                    .ok_or(FsError::OutOfMemory)?;
                write_lfn_and_83(state, sc, si, &name_owned, &short_name, 0x20, 0, 0)
                    .ok_or(FsError::IoError)?
            } else {
                let mut short_name = [b' '; 11];
                let nt_res = name_to_short(&name_owned, &mut short_name);
                let (sc, si) = find_free_dir_slot(state, dir_cluster)
                    .ok_or(FsError::OutOfMemory)?;
                let mut new_entry = Fat32DirEntry::default();
                new_entry.name   = short_name;
                new_entry.nt_res = nt_res;
                new_entry.attr   = 0x20;
                if !read_cluster(state, sc) { return Err(FsError::IoError); }
                let entry_bytes = unsafe {
                    core::slice::from_raw_parts(&new_entry as *const Fat32DirEntry as *const u8, 32)
                };
                state.cluster_buf[si as usize * 32..si as usize * 32 + 32].copy_from_slice(entry_bytes);
                if !write_cluster(state, sc) { return Err(FsError::IoError); }
                (sc, si)
            };

            Ok(Fat32FileInode::new(0, 0, entry_cluster_num, entry_index))
        }
    }

    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let dir_cluster = self.dir_cluster;
        let name_owned = alloc::string::String::from(name);
        let state_opt = unsafe { &mut *STATE.0.get() };
        let state = match state_opt.as_mut() {
            Some(s) if s.ready => s,
            _ => return Err(FsError::IoError),
        };
        {

            // Reject if a name already exists.
            if lookup_in_dir(state, dir_cluster, &name_owned).is_some() {
                return Err(FsError::AlreadyExists);
            }

            // Allocate a cluster for the new directory.
            let (new_cluster, _) = alloc_cluster(state, 0).ok_or(FsError::OutOfMemory)?;

            // Zero the new cluster.
            state.cluster_buf.fill(0);

            // Write '.' entry (points to itself).
            let mut dot_entry = Fat32DirEntry::default();
            dot_entry.name = *b".          ";
            dot_entry.attr = ATTR_DIRECTORY;
            dot_entry.fst_clus_lo = (new_cluster & 0xFFFF) as u16;
            dot_entry.fst_clus_hi = ((new_cluster >> 16) & 0xFFFF) as u16;
            let dot_bytes = unsafe {
                core::slice::from_raw_parts(&dot_entry as *const Fat32DirEntry as *const u8, 32)
            };
            state.cluster_buf[0..32].copy_from_slice(dot_bytes);

            // Write '..' entry (points to parent).
            let mut dotdot_entry = Fat32DirEntry::default();
            dotdot_entry.name = *b"..         ";
            dotdot_entry.attr = ATTR_DIRECTORY;
            dotdot_entry.fst_clus_lo = (dir_cluster & 0xFFFF) as u16;
            dotdot_entry.fst_clus_hi = ((dir_cluster >> 16) & 0xFFFF) as u16;
            let dotdot_bytes = unsafe {
                core::slice::from_raw_parts(&dotdot_entry as *const Fat32DirEntry as *const u8, 32)
            };
            state.cluster_buf[32..64].copy_from_slice(dotdot_bytes);

            if !write_cluster(state, new_cluster) { return Err(FsError::IoError); }

            // Add directory entry in the parent — with LFN if the name requires it.
            if needs_lfn(&name_owned) {
                let short_name = generate_short_alias(state, dir_cluster, &name_owned)
                    .ok_or(FsError::OutOfMemory)?;
                let n_slots  = lfn_entry_count(&name_owned) + 1;
                let (sc, si) = find_n_free_dir_slots(state, dir_cluster, n_slots)
                    .ok_or(FsError::OutOfMemory)?;
                write_lfn_and_83(state, sc, si, &name_owned, &short_name,
                                  ATTR_DIRECTORY, new_cluster, 0)
                    .ok_or(FsError::IoError)?;
            } else {
                let mut short_name = [b' '; 11];
                let nt_res = name_to_short(&name_owned, &mut short_name);
                let (sc, si) = find_free_dir_slot(state, dir_cluster)
                    .ok_or(FsError::OutOfMemory)?;
                let mut new_entry = Fat32DirEntry::default();
                new_entry.name        = short_name;
                new_entry.nt_res      = nt_res;
                new_entry.attr        = ATTR_DIRECTORY;
                new_entry.fst_clus_lo = (new_cluster & 0xFFFF) as u16;
                new_entry.fst_clus_hi = ((new_cluster >> 16) & 0xFFFF) as u16;
                if !read_cluster(state, sc) { return Err(FsError::IoError); }
                let entry_bytes = unsafe {
                    core::slice::from_raw_parts(&new_entry as *const Fat32DirEntry as *const u8, 32)
                };
                state.cluster_buf[si as usize * 32..si as usize * 32 + 32].copy_from_slice(entry_bytes);
                if !write_cluster(state, sc) { return Err(FsError::IoError); }
            }

            Ok(Fat32DirInode::new(new_cluster))
        }
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let dir_cluster = self.dir_cluster;
        let name_owned = alloc::string::String::from(name);
        let state_opt = unsafe { &mut *STATE.0.get() };
        let state = match state_opt.as_mut() {
            Some(s) if s.ready => s,
            _ => return Err(FsError::IoError),
        };
        {

            let found = lookup_in_dir(state, dir_cluster, &name_owned)
                .ok_or(FsError::NotFound)?;

            // Free the cluster chain for files (directories must be empty first,
            // but for v1.0 we allow removal regardless).
            let first_cluster = entry_cluster(&found.entry);
            if first_cluster >= FIRST_DATA_CLUSTER {
                free_chain(state, first_cluster);
            }

            // Delete any LFN entries that precede this 8.3 entry.
            if found.lfn_count > 0 {
                mark_entries_deleted(state, found.lfn_start_cluster, found.lfn_start_index,
                                     found.lfn_count);
            }

            // Mark the 8.3 directory entry as deleted.
            if !read_cluster(state, found.cluster) { return Err(FsError::IoError); }
            let offset = found.entry_index as usize * 32;
            if offset < state.cluster_buf.len() {
                state.cluster_buf[offset] = DIR_ENTRY_DELETED;
            }
            if !write_cluster(state, found.cluster) { return Err(FsError::IoError); }

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Fat32FileInode
// ---------------------------------------------------------------------------

/// VFS inode for a FAT32 regular file.
///
/// File position is per-FileDescriptor, not per-inode — the VFS layer
/// manages position in `FileDescriptor::InoFile { position }`.
///
/// `first_cluster` and `file_size` are mutable via `UnsafeCell` so that
/// `write_at` can update them in-place after each write.  This is safe
/// because the kernel runs single-core with IRQs disabled during syscalls —
/// no concurrent access to a single inode is possible.
pub struct Fat32FileInode {
    inode_number: u64,
    /// First cluster of the file's data chain.  Starts at 0 for a newly
    /// created file and is updated to the real cluster after the first write.
    first_cluster: UnsafeCell<u32>,
    /// Current logical file size in bytes.  Updated after every write.
    file_size: UnsafeCell<u32>,
    /// Directory cluster + entry index — needed for write (update dir entry size).
    dir_cluster:     u32,
    dir_entry_index: u32,
}

unsafe impl Send for Fat32FileInode {}
unsafe impl Sync for Fat32FileInode {}

impl Fat32FileInode {
    fn new(
        first_cluster: u32,
        file_size: u32,
        dir_cluster: u32,
        dir_entry_index: u32,
    ) -> Arc<dyn Inode> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            first_cluster: UnsafeCell::new(first_cluster),
            file_size: UnsafeCell::new(file_size),
            dir_cluster,
            dir_entry_index,
        })
    }
}

impl Inode for Fat32FileInode {
    fn inode_type(&self) -> InodeType {
        InodeType::RegularFile
    }

    fn stat(&self) -> InodeStat {
        let file_size = unsafe { *self.file_size.get() };
        InodeStat::regular(self.inode_number, file_size as u64)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let mut fat_file = Fat32File {
            dir_cluster:     self.dir_cluster,
            dir_entry_index: self.dir_entry_index,
            first_cluster:   unsafe { *self.first_cluster.get() },
            file_size:       unsafe { *self.file_size.get() },
            position:        offset,
        };
        let n = fat32_read(&mut fat_file, buf);
        if n < 0 { Err(FsError::IoError) } else { Ok(n as usize) }
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut fat_file = Fat32File {
            dir_cluster:     self.dir_cluster,
            dir_entry_index: self.dir_entry_index,
            first_cluster:   unsafe { *self.first_cluster.get() },
            file_size:       unsafe { *self.file_size.get() },
            position:        offset,
        };
        let n = fat32_write(&mut fat_file, buf);
        if n >= 0 {
            // Persist updated cluster chain head and file size back into
            // the inode so that subsequent writes through this same fd
            // extend the chain correctly instead of starting a new one.
            unsafe {
                *self.first_cluster.get() = fat_file.first_cluster;
                *self.file_size.get()     = fat_file.file_size;
            }
        }
        if n < 0 { Err(FsError::IoError) } else { Ok(n as usize) }
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> {
        None // not a directory
    }

    fn readdir(&self, _index: usize) -> Option<VfsDirEntry> {
        None // not a directory
    }

    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDirectory)
    }

    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDirectory)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotDirectory)
    }
}

// ---------------------------------------------------------------------------
// Public factory — called from kernel_main after fat32_init()
// ---------------------------------------------------------------------------

/// Return the volume label of the currently mounted FAT32 partition.
///
/// The label is the 11-byte space-padded value from BPB bytes 43–53.
/// Trailing spaces are stripped before returning.
/// Returns an empty string if FAT32 is not initialised.
pub fn fat32_volume_label() -> [u8; 11] {
    with_state(|state| state.volume_label)
}

/// Return the VFS root inode for the FAT32 filesystem.
///
/// Returns `None` if FAT32 is not initialised (no disk or bad format).
/// Mount the returned inode at `/mnt` via `vfs_mount("/mnt", root)`.
pub fn fat32_root_inode() -> Option<Arc<dyn Inode>> {
    let root_cluster = with_state(|state| {
        if state.ready { Some(state.root_cluster) } else { None }
    });
    root_cluster.map(Fat32DirInode::new)
}
