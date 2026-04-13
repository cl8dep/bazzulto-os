// fs/inode.rs — Inode abstraction for the VFS layer.
//
// Every file-system object (file, directory, device) implements `Inode`.
// The VFS layer holds `Arc<dyn Inode>` references so that multiple file
// descriptors can share the same underlying node (e.g. two `open()` calls
// on the same path both hold a reference to the same TmpfsFile).
//
// Design constraints:
//   - `no_std` — no OS services, no threads, single-core with IRQs disabled.
//   - Interior mutability via `UnsafeCell` (safe on single-core with IRQs off).
//   - `Send + Sync` asserted manually for the same reason.
//   - All methods take `&self` — mutation goes through the UnsafeCell.
//
// Reference: Linux fs/inode.c `struct inode`, VFS operations `inode_operations`.

extern crate alloc;

use alloc::sync::Arc;
use alloc::string::String;

// ---------------------------------------------------------------------------
// Global inode number counter
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_INODE_NUMBER: AtomicU64 = AtomicU64::new(1);

/// Allocate the next unique inode number.
pub fn alloc_inode_number() -> u64 {
    NEXT_INODE_NUMBER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// InodeType
// ---------------------------------------------------------------------------

/// The kind of filesystem object an Inode represents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InodeType {
    /// Regular file (readable and writable bytes).
    RegularFile,
    /// Directory (contains named child inodes).
    Directory,
    /// Character device (e.g. /dev/null, /dev/tty).
    CharDevice,
    /// Named pipe / FIFO.
    ///
    /// Reference: POSIX.1-2017 §10 (Pipes and FIFOs).
    Fifo,
    /// Symbolic link — `read_at()` returns the target path bytes.
    ///
    /// Reference: POSIX.1-2017 §4.14 (Symbolic Links).
    Symlink,
}

// ---------------------------------------------------------------------------
// InodeStat — metadata returned by stat()/lstat()
// ---------------------------------------------------------------------------

/// Per-inode metadata.  Layout maps to a simplified subset of POSIX `struct stat`.
///
/// Field layout (each word = u64):
///   [0]  inode number
///   [1]  file size in bytes (0 for directories and devices)
///   [2]  type + mode bits  (InodeType as u64 | permission bits)
///   [3]  link count
///
/// Reference: POSIX.1-2017 `sys/stat.h`, Linux `stat(2)`.
#[derive(Clone, Copy, Debug)]
pub struct InodeStat {
    pub inode_number: u64,
    pub size: u64,
    /// File type + permission bits packed into a u64.
    ///
    /// Bits [11:0]: POSIX mode bits (e.g. 0o755, 0o644).
    /// Bits [15:12]: file type (0=regular, 4=directory, 2=char device).
    /// Maps to `st_mode` in POSIX struct stat.
    pub mode: u64,
    pub nlinks: u64,
}

impl InodeStat {
    /// Construct stat for a regular file.
    pub fn regular(inode_number: u64, size: u64) -> Self {
        Self { inode_number, size, mode: 0o100644, nlinks: 1 }
    }

    /// Construct stat for a directory.
    pub fn directory(inode_number: u64) -> Self {
        Self { inode_number, size: 0, mode: 0o040755, nlinks: 2 }
    }

    /// Construct stat for a character device.
    pub fn char_device(inode_number: u64) -> Self {
        Self { inode_number, size: 0, mode: 0o020666, nlinks: 1 }
    }
}

// ---------------------------------------------------------------------------
// DirEntry — one entry returned by readdir()
// ---------------------------------------------------------------------------

/// A single directory entry (file name + type).
///
/// Returned by `Inode::readdir()` for directory inodes.
/// The `name` is a heap-allocated String to avoid lifetime issues.
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// File name (no path prefix).
    pub name: String,
    /// Type of this entry.
    pub inode_type: InodeType,
    /// Inode number of this entry.
    pub inode_number: u64,
}

// ---------------------------------------------------------------------------
// FsError
// ---------------------------------------------------------------------------

/// Error type for Inode operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FsError {
    /// Operation not valid for this inode type (e.g. read on a directory).
    NotSupported,
    /// Name not found in directory.
    NotFound,
    /// A component of the path is not a directory.
    NotDirectory,
    /// A file or directory with that name already exists.
    AlreadyExists,
    /// Directory is not empty (returned by rmdir).
    DirectoryNotEmpty,
    /// Out of memory.
    OutOfMemory,
    /// I/O error.
    IoError,
    /// Permission denied.
    PermissionDenied,
    /// Write to a pipe/FIFO with no readers (EPIPE).
    ///
    /// Reference: POSIX.1-2017 write(2), §2.7.
    BrokenPipe,
    /// Temporary unavailability; caller should retry (EAGAIN / EWOULDBLOCK).
    WouldBlock,
    /// Too many levels of symbolic links (ELOOP).
    ///
    /// Reference: POSIX.1-2017 §2.3.
    TooManyLinks,
    /// Invalid argument (EINVAL).
    InvalidArgument,
}

impl FsError {
    /// Convert to a POSIX-compatible negative errno value.
    pub fn to_errno(self) -> i64 {
        match self {
            FsError::NotSupported      => -38,  // ENOSYS
            FsError::NotFound          => -2,   // ENOENT
            FsError::NotDirectory      => -20,  // ENOTDIR
            FsError::AlreadyExists     => -17,  // EEXIST
            FsError::DirectoryNotEmpty => -39,  // ENOTEMPTY
            FsError::OutOfMemory       => -12,  // ENOMEM
            FsError::IoError           => -5,   // EIO
            FsError::PermissionDenied  => -1,   // EPERM
            FsError::BrokenPipe        => -32,  // EPIPE
            FsError::WouldBlock        => -11,  // EAGAIN / EWOULDBLOCK
            FsError::TooManyLinks      => -40,  // ELOOP
            FsError::InvalidArgument   => -22,  // EINVAL
        }
    }
}

// ---------------------------------------------------------------------------
// Inode trait
// ---------------------------------------------------------------------------

/// Core VFS interface.
///
/// All file-system objects implement this trait.  The VFS layer holds
/// `Arc<dyn Inode>` references and calls these methods without knowing the
/// concrete type.
///
/// Safety contract: all implementations use `UnsafeCell` for interior
/// mutability and assert `Send + Sync`.  Callers must ensure IRQs are
/// disabled and the kernel is single-core (no concurrent access).
pub trait Inode: Send + Sync {
    /// Return the type of this inode.
    fn inode_type(&self) -> InodeType;

    /// Return metadata for this inode.
    fn stat(&self) -> InodeStat;

    // --- Regular file operations ---

    /// Read bytes starting at `offset`.
    ///
    /// Returns the number of bytes actually read (0 = EOF).
    /// Returns `FsError::NotSupported` for non-regular-file inodes.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError>;

    /// Write bytes starting at `offset`.
    ///
    /// Extends the file if `offset + buf.len() > current size`.
    /// Returns the number of bytes written.
    /// Returns `FsError::NotSupported` for non-regular-file inodes.
    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError>;

    /// Truncate or extend the file to `new_size`.
    ///
    /// Returns `FsError::NotSupported` for non-regular-file inodes.
    fn truncate(&self, new_size: u64) -> Result<(), FsError>;

    // --- Directory operations ---

    /// Look up a child by name.
    ///
    /// Returns `None` if the name is not found or this is not a directory.
    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>>;

    /// Return the directory entry at position `index` (0-based).
    ///
    /// Used by `getdents64()`. Returns `None` when `index >= entry_count`.
    /// Returns `FsError::NotSupported` for non-directory inodes.
    fn readdir(&self, index: usize) -> Option<DirEntry>;

    /// Create a regular file named `name` in this directory.
    ///
    /// Returns the new inode on success.
    /// Returns `FsError::NotDirectory` if called on a non-directory.
    /// Returns `FsError::AlreadyExists` if the name is taken.
    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;

    /// Create a directory named `name` inside this directory.
    ///
    /// Returns the new inode on success.
    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;

    /// Remove the entry named `name` from this directory.
    ///
    /// Returns `FsError::DirectoryNotEmpty` if the target is a non-empty dir.
    fn unlink(&self, name: &str) -> Result<(), FsError>;

    /// Set the POSIX mode bits of this inode.
    ///
    /// Used by `open(O_CREAT, mode)` and `mkdir(mode)` to apply the
    /// caller-requested mode after creation (before umask masking).
    /// Default implementation is a no-op — filesystems that do not track
    /// per-inode mode (FAT32, devfs) silently ignore this call.
    fn set_mode(&self, _mode: u64) -> Result<(), FsError> {
        Ok(())
    }

    /// Insert an existing inode under `name` in this directory.
    ///
    /// Used by `rename()` to move an inode from one directory to another
    /// without re-creating it.  Default implementation returns `NotSupported`.
    fn link_child(&self, name: &str, child: Arc<dyn Inode>) -> Result<(), FsError> {
        let _ = (name, child);
        Err(FsError::NotSupported)
    }
}

// ---------------------------------------------------------------------------
// SymlinkInode — a symbolic link with a static target path
// ---------------------------------------------------------------------------

/// A symbolic link whose target is a fixed UTF-8 path string.
///
/// `read_at()` returns the target path bytes (no null terminator).
/// `stat()` returns mode `0o120777` (S_IFLNK | rwxrwxrwx).
///
/// Reference: POSIX.1-2017 §4.14 (Symbolic Links).
pub struct SymlinkInode {
    inode_number: u64,
    target: String,
}

unsafe impl Send for SymlinkInode {}
unsafe impl Sync for SymlinkInode {}

impl SymlinkInode {
    /// Create a new symbolic link inode pointing to `target`.
    pub fn new(target: String) -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            target,
        })
    }

    /// Return the symlink target as a string slice.
    pub fn target(&self) -> &str {
        &self.target
    }
}

impl Inode for SymlinkInode {
    fn inode_type(&self) -> InodeType { InodeType::Symlink }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: self.target.len() as u64,
            mode: 0o120777,   // S_IFLNK | rwxrwxrwx
            nlinks: 1,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let bytes = self.target.as_bytes();
        let start = offset as usize;
        if start >= bytes.len() { return Ok(0); }
        let count = (bytes.len() - start).min(buf.len());
        buf[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _offset: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }
    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }
    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }
}
