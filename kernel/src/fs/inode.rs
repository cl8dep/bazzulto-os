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
    /// Owner user ID.  Used by DAC permission checks.
    pub uid: u32,
    /// Owner group ID.
    pub gid: u32,
}

impl InodeStat {
    /// Construct stat for a regular file (owner root:root).
    pub fn regular(inode_number: u64, size: u64) -> Self {
        Self { inode_number, size, mode: 0o100644, nlinks: 1, uid: 0, gid: 0 }
    }

    /// Construct stat for a directory (owner root:root).
    pub fn directory(inode_number: u64) -> Self {
        Self { inode_number, size: 0, mode: 0o040755, nlinks: 2, uid: 0, gid: 0 }
    }

    /// Construct stat for a character device (owner root:root).
    pub fn char_device(inode_number: u64) -> Self {
        Self { inode_number, size: 0, mode: 0o020666, nlinks: 1, uid: 0, gid: 0 }
    }
}

// ---------------------------------------------------------------------------
// DAC access check — POSIX Discretionary Access Control
// ---------------------------------------------------------------------------

/// Access mode flags for `vfs_check_access`.
pub const ACCESS_READ:    u32 = 4; // R_OK
pub const ACCESS_WRITE:   u32 = 2; // W_OK
pub const ACCESS_EXECUTE: u32 = 1; // X_OK

/// Check POSIX DAC permissions for `process` accessing `inode`.
///
/// Returns `Ok(())` if access is allowed, `Err(FsError::PermissionDenied)` otherwise.
///
/// Rules (POSIX.1-2017 §2.7.1):
///   1. euid==0 (superuser): read/write always allowed.  Execute requires
///      at least one execute bit set (any of owner/group/other).
///   2. euid == inode.uid: use owner permission bits (mode >> 6).
///   3. egid == inode.gid OR inode.gid in supplementary groups: use group bits (mode >> 3).
///   4. Otherwise: use other permission bits (mode & 0o7).
///
/// Reference: POSIX.1-2017 §2.7.1, Linux VFS `inode_permission()`.
pub fn vfs_check_access(
    stat: &InodeStat,
    euid: u32,
    egid: u32,
    supplemental_groups: &[u32; 16],
    ngroups: usize,
    access: u32,
) -> Result<(), FsError> {
    // Rule 1: superuser bypass.
    if euid == 0 {
        // Even root needs at least one execute bit to execute a file.
        if access & ACCESS_EXECUTE != 0 {
            let any_x = stat.mode & 0o111;
            if any_x == 0 {
                return Err(FsError::PermissionDenied);
            }
        }
        return Ok(());
    }

    // Determine applicable permission bits.
    let mode_bits = if euid == stat.uid {
        // Owner.
        ((stat.mode >> 6) & 0o7) as u32
    } else if egid == stat.gid || group_match(stat.gid, supplemental_groups, ngroups) {
        // Group.
        ((stat.mode >> 3) & 0o7) as u32
    } else {
        // Other.
        (stat.mode & 0o7) as u32
    };

    // Check requested access against applicable bits.
    if access & mode_bits == access {
        Ok(())
    } else {
        Err(FsError::PermissionDenied)
    }
}

/// Check if `gid` is in the supplementary group list.
fn group_match(gid: u32, groups: &[u32; 16], ngroups: usize) -> bool {
    for i in 0..ngroups {
        if groups[i] == gid { return true; }
    }
    false
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

    /// Return `true` if this inode is exec-restricted to the kernel only.
    ///
    /// When `true`, `sys_exec()` returns `EPERM` for any process that attempts
    /// to `exec` this file.  The kernel itself may still read and exec the file
    /// during boot (e.g. spawning bzinit as PID 1).
    ///
    /// Used to protect `bzinit` from being exec'd by arbitrary userspace processes.
    ///
    /// Default implementation returns `false`.
    ///
    /// Reference: docs/features/Binary Permission Model.md §INODE_KERNEL_EXEC_ONLY.
    fn is_kernel_exec_only(&self) -> bool {
        false
    }

    /// Return the IPC type discriminant and table index for this inode.
    ///
    /// Overridden by IPC inodes (semaphore=1, socket=2, mqueue=3) to expose
    /// the table index for syscall handlers without encoding it in `nlinks`.
    ///
    /// Default returns `None` — non-IPC inodes do not have table indices.
    fn ipc_table_index(&self) -> Option<(u8, usize)> {
        None
    }

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

    /// Flush all dirty data for this file to the underlying storage.
    ///
    /// Default implementation returns `Ok(())` — filesystems that maintain
    /// write-back caches (e.g. FAT32 cluster cache) should override this.
    ///
    /// Reference: POSIX.1-2017 fsync(2).
    fn fsync(&self) -> Result<(), FsError> {
        Ok(())
    }

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
    /// Used by `chmod()`, `fchmod()`, and `open(O_CREAT, mode)` to set
    /// permission bits.  Default implementation is a no-op — filesystems that
    /// do not track per-inode mode (FAT32, devfs) silently ignore this call.
    fn set_mode(&self, _mode: u64) -> Result<(), FsError> {
        Ok(())
    }

    /// Set the owner (uid, gid) of this inode.
    ///
    /// Used by `chown()` and `fchown()`.  Pass `u32::MAX` for either field
    /// to leave it unchanged (matching POSIX chown semantics where -1 = no change).
    /// Default implementation is a no-op — filesystems without ownership
    /// tracking (FAT32, devfs) silently ignore this call.
    fn set_owner(&self, _uid: u32, _gid: u32) -> Result<(), FsError> {
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

    /// Return the FAT32 first-cluster number for this inode, if applicable.
    ///
    /// Only meaningful for `Fat32FileInode` and `Fat32DirInode`.  Used by
    /// `Fat32DirInode::link_child()` to write the cluster pointer into the new
    /// directory entry when performing a rename.
    ///
    /// Default returns `None` (not a FAT32 inode).
    fn fat32_first_cluster(&self) -> Option<u32> {
        None
    }

    /// Return filesystem block statistics for the volume this inode belongs to.
    ///
    /// Returns `Some((total_512_blocks, free_512_blocks))` when the filesystem
    /// supports block accounting (e.g. FAT32).  Returns `None` for virtual
    /// filesystems (tmpfs, devfs, procfs) that do not track block usage.
    ///
    /// Used by `sys_getmounts` to populate `df`-style output.
    ///
    /// Default implementation returns `None`.
    fn fs_stats(&self) -> Option<(u64, u64)> {
        None
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
            uid: 0,
            gid: 0,
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

// ---------------------------------------------------------------------------
// KernelExecOnlyInode — wraps any inode and marks it exec-restricted
// ---------------------------------------------------------------------------

/// Wraps an `Arc<dyn Inode>` and overrides `is_kernel_exec_only()` to return
/// `true`, blocking any userspace `exec()` attempt on this file.
///
/// All other `Inode` methods delegate directly to the inner inode.
///
/// Usage: wrap the bzinit inode in `vfs_init()` before mounting it so that
/// no userspace process can accidentally re-exec bzinit.
///
/// Reference: docs/features/Binary Permission Model.md §INODE_KERNEL_EXEC_ONLY.
pub struct KernelExecOnlyInode {
    inner: Arc<dyn Inode>,
}

unsafe impl Send for KernelExecOnlyInode {}
unsafe impl Sync for KernelExecOnlyInode {}

impl KernelExecOnlyInode {
    /// Wrap `inner` so that `is_kernel_exec_only()` returns `true`.
    pub fn wrap(inner: Arc<dyn Inode>) -> Arc<dyn Inode> {
        Arc::new(KernelExecOnlyInode { inner })
    }
}

impl Inode for KernelExecOnlyInode {
    fn inode_type(&self) -> InodeType { self.inner.inode_type() }
    fn stat(&self) -> InodeStat { self.inner.stat() }
    fn is_kernel_exec_only(&self) -> bool { true }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        self.inner.read_at(offset, buf)
    }
    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        self.inner.write_at(offset, buf)
    }
    fn truncate(&self, new_size: u64) -> Result<(), FsError> {
        self.inner.truncate(new_size)
    }
    fn fsync(&self) -> Result<(), FsError> { self.inner.fsync() }
    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> { self.inner.lookup(name) }
    fn readdir(&self, index: usize) -> Option<DirEntry> { self.inner.readdir(index) }
    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> { self.inner.create(name) }
    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> { self.inner.mkdir(name) }
    fn unlink(&self, name: &str) -> Result<(), FsError> { self.inner.unlink(name) }
    fn set_mode(&self, mode: u64) -> Result<(), FsError> { self.inner.set_mode(mode) }
    fn link_child(&self, name: &str, child: Arc<dyn Inode>) -> Result<(), FsError> {
        self.inner.link_child(name, child)
    }
    fn fat32_first_cluster(&self) -> Option<u32> { self.inner.fat32_first_cluster() }
}
