// fs/tmpfs.rs — In-memory filesystem with real directory structure.
//
// Replaces the flat `ramfs` for general use.  Supports subdirectories,
// creation, deletion, and rename.  All data lives in kernel heap memory.
//
// Nodes:
//   TmpfsDir  — a directory; children stored in a BTreeMap<String, Arc<dyn Inode>>.
//   TmpfsFile — a regular file; data stored in a Vec<u8>.
//
// Interior mutability via UnsafeCell.  Safe on single-core with IRQs off.
//
// Reference: Linux mm/shmem.c (tmpfs), Plan 9 ramfs.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use super::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};

// ---------------------------------------------------------------------------
// TmpfsFile — regular file
// ---------------------------------------------------------------------------

struct TmpfsFileInner {
    inode_number: u64,
    data: Vec<u8>,
    mode: u64,
}

pub struct TmpfsFile(UnsafeCell<TmpfsFileInner>);

// SAFETY: single-core kernel; all access is serialized by IRQ disable.
unsafe impl Send for TmpfsFile {}
unsafe impl Sync for TmpfsFile {}

impl TmpfsFile {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(UnsafeCell::new(TmpfsFileInner {
            inode_number: alloc_inode_number(),
            data: Vec::new(),
            mode: 0o100644,
        })))
    }

    /// Create a TmpfsFile pre-populated with `data`.
    pub fn new_with_data(data: &[u8]) -> Arc<Self> {
        let file = Self::new();
        unsafe {
            let inner = &mut *file.0.get();
            inner.data.extend_from_slice(data);
        }
        file
    }

    /// Create a TmpfsFile with an explicit POSIX mode (type bits + permission bits).
    pub fn new_with_mode(mode: u64) -> Arc<Self> {
        Arc::new(Self(UnsafeCell::new(TmpfsFileInner {
            inode_number: alloc_inode_number(),
            data: Vec::new(),
            mode,
        })))
    }

    #[inline]
    unsafe fn inner(&self) -> &mut TmpfsFileInner {
        &mut *self.0.get()
    }
}

impl Inode for TmpfsFile {
    fn inode_type(&self) -> InodeType { InodeType::RegularFile }

    fn stat(&self) -> InodeStat {
        let inner = unsafe { self.inner() };
        let mut stat = InodeStat::regular(inner.inode_number, inner.data.len() as u64);
        stat.mode = inner.mode;
        stat
    }

    fn set_mode(&self, mode: u64) -> Result<(), FsError> {
        let inner = unsafe { self.inner() };
        inner.mode = mode;
        Ok(())
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = unsafe { self.inner() };
        let data = &inner.data;
        let start = offset as usize;
        if start >= data.len() {
            return Ok(0); // EOF
        }
        let available = data.len() - start;
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&data[start..start + to_read]);
        Ok(to_read)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let inner = unsafe { self.inner() };
        let start = offset as usize;
        let end = start + buf.len();
        // Extend if necessary.
        if end > inner.data.len() {
            inner.data.resize(end, 0);
        }
        inner.data[start..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn truncate(&self, new_size: u64) -> Result<(), FsError> {
        let inner = unsafe { self.inner() };
        inner.data.resize(new_size as usize, 0);
        Ok(())
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }

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
// TmpfsDir — directory
// ---------------------------------------------------------------------------

struct TmpfsDirInner {
    inode_number: u64,
    entries: BTreeMap<String, Arc<dyn Inode>>,
    mode: u64,
}

pub struct TmpfsDir(UnsafeCell<TmpfsDirInner>);

// SAFETY: single-core kernel; all access is serialized by IRQ disable.
unsafe impl Send for TmpfsDir {}
unsafe impl Sync for TmpfsDir {}

impl TmpfsDir {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(UnsafeCell::new(TmpfsDirInner {
            inode_number: alloc_inode_number(),
            entries: BTreeMap::new(),
            mode: 0o040755,
        })))
    }

    /// Create a TmpfsDir with an explicit POSIX mode (type bits + permission bits).
    pub fn new_with_mode(mode: u64) -> Arc<Self> {
        Arc::new(Self(UnsafeCell::new(TmpfsDirInner {
            inode_number: alloc_inode_number(),
            entries: BTreeMap::new(),
            mode,
        })))
    }

    /// Insert a child inode directly (used during VFS initialisation).
    pub fn insert(&self, name: &str, inode: Arc<dyn Inode>) {
        let inner = unsafe { &mut *self.0.get() };
        inner.entries.insert(name.to_string(), inode);
    }

    #[inline]
    unsafe fn inner(&self) -> &mut TmpfsDirInner {
        &mut *self.0.get()
    }
}

impl Inode for TmpfsDir {
    fn inode_type(&self) -> InodeType { InodeType::Directory }

    fn stat(&self) -> InodeStat {
        let inner = unsafe { self.inner() };
        let mut stat = InodeStat::directory(inner.inode_number);
        stat.mode = inner.mode;
        stat
    }

    fn set_mode(&self, mode: u64) -> Result<(), FsError> {
        let inner = unsafe { self.inner() };
        inner.mode = mode;
        Ok(())
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn truncate(&self, _size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let inner = unsafe { self.inner() };
        inner.entries.get(name).cloned()
    }

    fn readdir(&self, index: usize) -> Option<DirEntry> {
        let inner = unsafe { self.inner() };
        // Synthetic "." and ".." entries at indices 0 and 1.
        match index {
            0 => return Some(DirEntry {
                name: ".".to_string(),
                inode_type: InodeType::Directory,
                inode_number: inner.inode_number,
            }),
            1 => return Some(DirEntry {
                name: "..".to_string(),
                inode_type: InodeType::Directory,
                inode_number: inner.inode_number, // simplified: parent ino unknown
            }),
            _ => {}
        }
        let real_index = index - 2;
        inner.entries.iter().nth(real_index).map(|(name, inode)| DirEntry {
            name: name.clone(),
            inode_type: inode.inode_type(),
            inode_number: inode.stat().inode_number,
        })
    }

    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let inner = unsafe { self.inner() };
        if inner.entries.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }
        let file = TmpfsFile::new();
        inner.entries.insert(name.to_string(), file.clone());
        Ok(file)
    }

    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let inner = unsafe { self.inner() };
        if inner.entries.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }
        let dir = TmpfsDir::new();
        inner.entries.insert(name.to_string(), dir.clone());
        Ok(dir)
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let inner = unsafe { self.inner() };
        let target = inner.entries.get(name).ok_or(FsError::NotFound)?;
        // Refuse to remove a non-empty directory.
        if target.inode_type() == InodeType::Directory {
            if target.readdir(2).is_some() {
                return Err(FsError::DirectoryNotEmpty);
            }
        }
        inner.entries.remove(name);
        Ok(())
    }

    fn link_child(&self, name: &str, child: Arc<dyn Inode>) -> Result<(), FsError> {
        let inner = unsafe { self.inner() };
        inner.entries.insert(name.to_string(), child);
        Ok(())
    }
}
