// fs/ramfs.rs — Read-only in-memory file system.
//
// Backed by static byte slices embedded in the kernel binary.
// Supports up to 128 files, names up to 255 bytes.
//
// Design: a flat array of RamFsEntry values.  No directories — all files
// live in a single root namespace.  This matches the C kernel's ramfs design.

/// Maximum number of files in the ramfs.
pub const RAMFS_MAX_FILES: usize = 128;

/// Maximum length of a file name (excluding the NUL terminator).
pub const RAMFS_MAX_NAME_LENGTH: usize = 255;

/// One file entry.
#[derive(Clone, Copy)]
pub struct RamFsEntry {
    /// File name as a UTF-8 string.  Empty name = unused slot.
    pub name: [u8; RAMFS_MAX_NAME_LENGTH + 1], // +1 for NUL sentinel
    /// Pointer to the file's data.
    pub data: &'static [u8],
}

impl RamFsEntry {
    const fn empty() -> Self {
        Self {
            name: [0u8; RAMFS_MAX_NAME_LENGTH + 1],
            data: &[],
        }
    }

    fn name_str(&self) -> &str {
        let length = self.name.iter().position(|&byte| byte == 0).unwrap_or(RAMFS_MAX_NAME_LENGTH);
        core::str::from_utf8(&self.name[..length]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Global ramfs table
// ---------------------------------------------------------------------------

static mut RAMFS_TABLE: [RamFsEntry; RAMFS_MAX_FILES] = [RamFsEntry::empty(); RAMFS_MAX_FILES];
static mut RAMFS_FILE_COUNT: usize = 0;

/// Register a static file in the ramfs.
///
/// # Safety
/// Must be called before any user process accesses the ramfs
/// (i.e., during kernel initialisation, single-threaded, IRQs off).
pub unsafe fn ramfs_register_file(name: &str, data: &'static [u8]) -> bool {
    if RAMFS_FILE_COUNT >= RAMFS_MAX_FILES {
        return false;
    }
    if name.len() > RAMFS_MAX_NAME_LENGTH {
        return false;
    }
    let entry = &mut RAMFS_TABLE[RAMFS_FILE_COUNT];
    let name_bytes = name.as_bytes();
    entry.name[..name_bytes.len()].copy_from_slice(name_bytes);
    entry.name[name_bytes.len()] = 0;
    entry.data = data;
    RAMFS_FILE_COUNT += 1;
    true
}

/// Find a file by name.  Returns a `&'static [u8]` slice if found.
pub fn ramfs_find(name: &str) -> Option<&'static [u8]> {
    let count = unsafe { RAMFS_FILE_COUNT };
    for index in 0..count {
        let entry = unsafe { &RAMFS_TABLE[index] };
        if entry.name_str() == name {
            return Some(entry.data);
        }
    }
    None
}

/// List all files: calls `callback(name)` for each entry.
pub fn ramfs_list<F: FnMut(&str)>(mut callback: F) {
    let count = unsafe { RAMFS_FILE_COUNT };
    for index in 0..count {
        let entry = unsafe { &RAMFS_TABLE[index] };
        callback(entry.name_str());
    }
}
