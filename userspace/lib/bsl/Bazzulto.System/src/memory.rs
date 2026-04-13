//! Memory mapping and allocation wrappers.

use core::alloc::Layout;
use crate::raw;

// ---------------------------------------------------------------------------
// Protection flags
// ---------------------------------------------------------------------------

/// Memory protection flags.
#[derive(Clone, Copy)]
pub struct Protection(i32);

impl Protection {
    pub const READ:    Protection = Protection(0x1);
    pub const WRITE:   Protection = Protection(0x2);
    pub const EXECUTE: Protection = Protection(0x4);
}

impl core::ops::BitOr for Protection {
    type Output = Protection;
    fn bitor(self, rhs: Protection) -> Protection {
        Protection(self.0 | rhs.0)
    }
}

// ---------------------------------------------------------------------------
// Map flags
// ---------------------------------------------------------------------------

/// Memory map flags.
#[derive(Clone, Copy)]
pub struct MapFlags(i32);

impl MapFlags {
    pub const ANONYMOUS: MapFlags = MapFlags(0x20);
    pub const PRIVATE:   MapFlags = MapFlags(0x02);
    pub const SHARED:    MapFlags = MapFlags(0x01);
}

impl core::ops::BitOr for MapFlags {
    type Output = MapFlags;
    fn bitor(self, rhs: MapFlags) -> MapFlags {
        MapFlags(self.0 | rhs.0)
    }
}

// ---------------------------------------------------------------------------
// MemoryMap — RAII owner of a mapped region
// ---------------------------------------------------------------------------

/// RAII owner of a mapped memory region.
pub struct MemoryMap {
    base: *mut u8,
    length: usize,
}

impl MemoryMap {
    pub fn valid(&self) -> bool {
        !self.base.is_null()
    }

    pub fn data(&self) -> *mut u8 {
        self.base
    }

    pub fn size(&self) -> usize {
        self.length
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.base, self.length) }
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.base, self.length) }
    }
}

impl Drop for MemoryMap {
    fn drop(&mut self) {
        if !self.base.is_null() {
            raw::raw_munmap(self.base as u64, self.length as u64);
        }
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

pub struct Memory;

impl Memory {
    /// Allocate `size` bytes via the global allocator.
    pub fn alloc(size: usize) -> *mut u8 {
        let layout = match Layout::from_size_align(size, 8) {
            Ok(l) => l,
            Err(_) => return core::ptr::null_mut(),
        };
        unsafe { alloc::alloc::alloc(layout) }
    }

    /// Free a pointer allocated with `Memory::alloc`.
    pub fn free(ptr: *mut u8, size: usize) {
        if ptr.is_null() {
            return;
        }
        let layout = match Layout::from_size_align(size, 8) {
            Ok(l) => l,
            Err(_) => return,
        };
        unsafe { alloc::alloc::dealloc(ptr, layout) };
    }

    /// Reallocate a pointer to a new size.
    pub fn realloc(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
        let layout = match Layout::from_size_align(old_size, 8) {
            Ok(l) => l,
            Err(_) => return core::ptr::null_mut(),
        };
        unsafe { alloc::alloc::realloc(ptr, layout, new_size) }
    }

    /// Map anonymous memory with the given protection and flags.
    pub fn map(
        addr: Option<usize>,
        size: usize,
        prot: Protection,
        flags: MapFlags,
    ) -> Result<MemoryMap, i32> {
        let hint = addr.unwrap_or(0) as u64;
        let result = raw::raw_mmap(hint, size as u64, prot.0, flags.0);
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(MemoryMap {
                base: result as usize as *mut u8,
                length: size,
            })
        }
    }

    /// Page size on AArch64.
    pub fn page_size() -> usize {
        4096
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn usage() -> Option<u64> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn available() -> Option<u64> {
        None
    }
}
