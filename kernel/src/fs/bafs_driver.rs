// fs/bafs_driver.rs — BAFS filesystem driver for the Bazzulto kernel VFS.
//
// This module bridges two independent type systems:
//
//   1. The kernel's `hal::disk::BlockDevice` trait (used everywhere in the
//      kernel for disk I/O).
//   2. The BAFS crate's own `bafs::block_device::BlockDevice` trait (defined
//      standalone so the BAFS crate compiles without depending on the kernel).
//
// The bridge is a `KernelBlockDeviceAdapter` that wraps an
// `Arc<dyn kernel_BlockDevice>` and forwards every I/O call, applying a
// fixed `start_lba` partition offset so that BAFS addresses sector 0 of the
// partition, not sector 0 of the physical disk.
//
// Once the adapter satisfies `bafs::BlockDevice`, a
// `BafsVolume<KernelBlockDeviceAdapter>` can be mounted and shared across
// inodes via `Arc<SpinLock<…>>`.
//
// Each VFS inode on a BAFS volume is represented by a `BafsVolumeInode` which
// holds:
//   - a shared reference to the volume lock,
//   - the 64-bit BAFS inode number of the file-system object it represents.
//
// `BafsVolumeInode` implements the kernel's `crate::fs::inode::Inode` trait,
// so the VFS layer can mount BAFS volumes exactly like FAT32 or tmpfs.
//
// Reference: BAFS on-disk format, `kernel/src/fs/bafs/src/volume.rs`.

extern crate alloc;

use alloc::sync::Arc;

use bafs::block_device::BlockDevice as BafsBlockDevice;
use bafs::error::BafsError;
use bafs::volume::{
    flush_and_commit,
    volume_create_directory,
    volume_create_file,
    volume_lookup_directory_entry,
    volume_read_directory_entry_at_index,
    volume_read_file_data,
    volume_read_inode,
    volume_unlink_directory_entry,
    volume_write_file_data,
    BafsVolume,
};
use bafs::dir::CHILD_TYPE_DIRECTORY;

use crate::fs::inode::{
    alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType,
};
use crate::hal::disk::BlockDevice as KernelBlockDevice;
use crate::sync::SpinLock;

// ---------------------------------------------------------------------------
// KernelBlockDeviceAdapter — forward kernel I/O to the BAFS trait
//
// Includes a `start_lba` partition offset so that BAFS block addresses are
// relative to the partition start, not the physical disk start.
// ---------------------------------------------------------------------------

/// Adapts an `Arc<dyn kernel::hal::BlockDevice>` to the `bafs::BlockDevice`
/// trait.  All sector addresses are offset by `partition_start_lba`.
struct KernelBlockDeviceAdapter {
    inner: Arc<dyn KernelBlockDevice>,
    /// First LBA of the partition on the physical disk.
    partition_start_lba: u64,
}

// Safety: the underlying `Arc<dyn BlockDevice>` is already `Send + Sync`.
unsafe impl Send for KernelBlockDeviceAdapter {}
unsafe impl Sync for KernelBlockDeviceAdapter {}

impl BafsBlockDevice for KernelBlockDeviceAdapter {
    fn read_sectors(
        &self,
        start_lba: u64,
        sector_count: u32,
        destination_buffer: &mut [u8],
    ) -> bool {
        self.inner.read_sectors(
            self.partition_start_lba + start_lba,
            sector_count,
            destination_buffer,
        )
    }

    fn write_sectors(
        &self,
        start_lba: u64,
        sector_count: u32,
        source_buffer: &[u8],
    ) -> bool {
        self.inner.write_sectors(
            self.partition_start_lba + start_lba,
            sector_count,
            source_buffer,
        )
    }

    fn total_sector_count(&self) -> u64 {
        self.inner.sector_count().saturating_sub(self.partition_start_lba)
    }

    fn sector_size_in_bytes(&self) -> u32 {
        self.inner.sector_size()
    }

    fn device_name(&self) -> &str {
        self.inner.name()
    }
}

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------

/// Concrete BAFS volume type used inside the kernel.
type KernelBafsVolume = BafsVolume<KernelBlockDeviceAdapter>;

// ---------------------------------------------------------------------------
// Map BafsError to FsError
// ---------------------------------------------------------------------------

fn map_error(error: BafsError) -> FsError {
    match error {
        BafsError::NotFound              => FsError::NotFound,
        BafsError::AlreadyExists         => FsError::AlreadyExists,
        BafsError::NotADirectory         => FsError::NotDirectory,
        BafsError::NotARegularFile       => FsError::NotSupported,
        BafsError::OutOfSpace            => FsError::OutOfMemory,
        BafsError::InvalidArgument       => FsError::InvalidArgument,
        BafsError::InputOutputError      => FsError::IoError,
        BafsError::InvalidChecksum { .. } => FsError::IoError,
        BafsError::CorruptedStructure    => FsError::IoError,
        _                                => FsError::IoError,
    }
}

// ---------------------------------------------------------------------------
// BafsVolumeInode — kernel VFS inode backed by a BAFS volume
// ---------------------------------------------------------------------------

/// A kernel VFS inode that delegates all operations to a shared `BafsVolume`.
///
/// Multiple `BafsVolumeInode` instances share the same
/// `Arc<SpinLock<KernelBafsVolume>>`.  The spinlock serialises all VFS
/// operations on that volume.  This matches the locking pattern used by
/// `fat32.rs`.
pub struct BafsVolumeInode {
    /// VFS-layer inode number (allocated from the global counter, distinct
    /// from the BAFS on-disk inode number).
    vfs_inode_number: u64,
    /// BAFS on-disk inode number of the file-system object this VFS inode
    /// represents.
    bafs_inode_number: u64,
    /// Shared reference to the mounted BAFS volume.
    volume: Arc<SpinLock<KernelBafsVolume>>,
}

unsafe impl Send for BafsVolumeInode {}
unsafe impl Sync for BafsVolumeInode {}

impl BafsVolumeInode {
    fn new(
        bafs_inode_number: u64,
        volume: Arc<SpinLock<KernelBafsVolume>>,
    ) -> Arc<dyn Inode> {
        Arc::new(BafsVolumeInode {
            vfs_inode_number: alloc_inode_number(),
            bafs_inode_number,
            volume,
        })
    }

    /// Construct an `Arc<dyn Inode>` for another inode on the same volume.
    fn make_child(&self, child_bafs_inode_number: u64) -> Arc<dyn Inode> {
        BafsVolumeInode::new(child_bafs_inode_number, Arc::clone(&self.volume))
    }
}

impl Inode for BafsVolumeInode {
    fn inode_type(&self) -> InodeType {
        let volume = self.volume.lock();
        match volume_read_inode(&*volume, self.bafs_inode_number) {
            Ok(inode) if inode.is_directory() => InodeType::Directory,
            _ => InodeType::RegularFile,
        }
    }

    fn stat(&self) -> InodeStat {
        let volume = self.volume.lock();
        match volume_read_inode(&*volume, self.bafs_inode_number) {
            Ok(inode) => {
                if inode.is_directory() {
                    InodeStat::directory(self.vfs_inode_number)
                } else {
                    InodeStat::regular(self.vfs_inode_number, inode.file_size_in_bytes)
                }
            }
            Err(_) => InodeStat::regular(self.vfs_inode_number, 0),
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let volume = self.volume.lock();
        volume_read_file_data(&*volume, self.bafs_inode_number, offset, buf)
            .map_err(map_error)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut volume = self.volume.lock();
        volume_write_file_data(&mut *volume, self.bafs_inode_number, offset, buf)
            .map_err(map_error)
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        // Truncation requires freeing extents — deferred to post-v1.0.
        // Reference: docs/DEBT.md "BAFS truncate (extent freeing)".
        Err(FsError::NotSupported)
    }

    fn fsync(&self) -> Result<(), FsError> {
        let mut volume = self.volume.lock();
        flush_and_commit(&mut *volume).map_err(map_error)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let volume = self.volume.lock();
        match volume_lookup_directory_entry(&*volume, self.bafs_inode_number, name) {
            Ok(Some(child_number)) => Some(self.make_child(child_number)),
            _ => None,
        }
    }

    fn readdir(&self, index: usize) -> Option<DirEntry> {
        let volume = self.volume.lock();
        match volume_read_directory_entry_at_index(
            &*volume,
            self.bafs_inode_number,
            index,
        ) {
            Ok(Some(entry)) => {
                let inode_type = if entry.child_type == CHILD_TYPE_DIRECTORY {
                    InodeType::Directory
                } else {
                    InodeType::RegularFile
                };
                Some(DirEntry {
                    name: entry.filename,
                    inode_type,
                    inode_number: entry.child_inode_number,
                })
            }
            _ => None,
        }
    }

    fn create(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let mut volume = self.volume.lock();
        let new_number =
            volume_create_file(&mut *volume, self.bafs_inode_number, name)
                .map_err(map_error)?;
        drop(volume);
        Ok(self.make_child(new_number))
    }

    fn mkdir(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let mut volume = self.volume.lock();
        let new_number =
            volume_create_directory(&mut *volume, self.bafs_inode_number, name)
                .map_err(map_error)?;
        drop(volume);
        Ok(self.make_child(new_number))
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let mut volume = self.volume.lock();
        volume_unlink_directory_entry(&mut *volume, self.bafs_inode_number, name)
            .map_err(map_error)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Probe `device` at `start_lba` for a BAFS superblock magic number.
///
/// Returns `true` if the first 8 bytes of the partition are the BAFS magic
/// `b"BAFS\x1B\x00\x00\x00"`.  The check is cheap (one sector read) and is
/// intended to be called before attempting a full mount.
///
/// Reference: BAFS on-disk format §2 "Superblock layout",
/// `kernel/src/fs/bafs/src/superblock.rs` `BAFS_MAGIC_NUMBER`.
pub fn bafs_probe(device: &Arc<dyn KernelBlockDevice>, start_lba: u64) -> bool {
    const BAFS_MAGIC: [u8; 8] = *b"BAFS\x1B\x00\x00\x00";
    let mut sector_buf = [0u8; 512];
    if !device.read_sectors(start_lba, 1, &mut sector_buf) {
        return false;
    }
    sector_buf[..8] == BAFS_MAGIC
}

/// Mount a BAFS filesystem from `device` starting at partition `start_lba`.
///
/// Returns the root directory inode on success, or `None` if the superblock is
/// corrupt, the magic is wrong, or journal recovery fails.
pub fn bafs_mount_partition(
    device: Arc<dyn KernelBlockDevice>,
    start_lba: u64,
) -> Option<Arc<dyn Inode>> {
    let adapter = KernelBlockDeviceAdapter { inner: device, partition_start_lba: start_lba };
    let volume = bafs::volume::bafs_mount(adapter).ok()?;
    let root_inode_number = volume.superblock.root_inode_number;
    let shared = Arc::new(SpinLock::new(volume));
    Some(BafsVolumeInode::new(root_inode_number, shared))
}
