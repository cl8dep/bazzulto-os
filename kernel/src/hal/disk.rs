// hal/disk.rs — Block device HAL: BlockDevice trait, global disk registry,
// and a thin wrapper around platform/qemu_virt/virtio_blk.rs.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// BlockDevice trait
// ---------------------------------------------------------------------------

/// Abstract interface for a block (sector-addressable) storage device.
///
/// All methods are `&self` — implementations use interior mutability
/// (single-core kernel with IRQs disabled).
///
/// Reference: Linux `include/linux/blkdev.h` `struct block_device`.
pub trait BlockDevice: Send + Sync {
    /// Read `count` 512-byte sectors starting at `lba` into `buf`.
    ///
    /// `buf` must be at least `count * 512` bytes.
    /// Returns `true` on success.
    fn read_sectors(&self, lba: u64, count: u32, buf: &mut [u8]) -> bool;

    /// Write `count` 512-byte sectors starting at `lba` from `buf`.
    ///
    /// `buf` must be at least `count * 512` bytes.
    /// Returns `true` on success.
    fn write_sectors(&self, lba: u64, count: u32, buf: &[u8]) -> bool;

    /// Total device capacity in 512-byte sectors.
    fn sector_count(&self) -> u64;

    /// Sector size in bytes (always 512 for the current virtio-blk implementation).
    fn sector_size(&self) -> u32 { 512 }

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// VirtioBlkDevice — wraps the virtio-blk platform functions
// ---------------------------------------------------------------------------

/// `BlockDevice` implementation backed by the QEMU virt virtio-blk device.
///
/// All I/O is delegated to `platform::qemu_virt::virtio_blk`.
/// Safety: single-core kernel with IRQs disabled at call sites.
pub struct VirtioBlkDevice;

unsafe impl Send for VirtioBlkDevice {}
unsafe impl Sync for VirtioBlkDevice {}

impl BlockDevice for VirtioBlkDevice {
    fn read_sectors(&self, lba: u64, count: u32, buf: &mut [u8]) -> bool {
        unsafe { crate::platform::qemu_virt::virtio_blk::disk_read_sectors(lba, count, buf) }
    }

    fn write_sectors(&self, lba: u64, count: u32, buf: &[u8]) -> bool {
        unsafe { crate::platform::qemu_virt::virtio_blk::disk_write_sectors(lba, count, buf) }
    }

    fn sector_count(&self) -> u64 {
        crate::platform::qemu_virt::virtio_blk::disk_capacity()
    }

    fn name(&self) -> &str { "virtio-blk0" }
}

// ---------------------------------------------------------------------------
// Global disk registry
// ---------------------------------------------------------------------------

struct DiskRegistry(UnsafeCell<Vec<Arc<dyn BlockDevice>>>);
unsafe impl Sync for DiskRegistry {}

static DISK_REGISTRY: DiskRegistry = DiskRegistry(UnsafeCell::new(Vec::new()));

/// Register a block device.
///
/// Called from `platform_init()` after virtio enumeration.
/// # Safety
/// Must be called single-threaded with IRQs disabled.
pub unsafe fn register_disk(device: Arc<dyn BlockDevice>) {
    let registry = &mut *DISK_REGISTRY.0.get();
    registry.push(device);
}

/// Return the number of registered block devices.
pub fn disk_count() -> usize {
    let registry = unsafe { &*DISK_REGISTRY.0.get() };
    registry.len()
}

/// Return the block device at `index`, or `None` if out of range.
pub fn get_disk(index: usize) -> Option<Arc<dyn BlockDevice>> {
    let registry = unsafe { &*DISK_REGISTRY.0.get() };
    registry.get(index).cloned()
}

// ---------------------------------------------------------------------------
// Legacy free functions — kept for callers that have not been updated
// ---------------------------------------------------------------------------

/// Initialise the virtio-blk device and register it in the disk registry.
///
/// # Safety
/// Must be called once during kernel boot with IRQs disabled.
pub unsafe fn init(hhdm_offset: u64) {
    crate::platform::qemu_virt::virtio_blk::disk_init(hhdm_offset);
    // Register the virtio-blk device so the rest of the kernel can reach it
    // through the BlockDevice trait without knowing the concrete type.
    let device: Arc<dyn BlockDevice> = Arc::new(VirtioBlkDevice);
    register_disk(device);
}

/// Read `count` 512-byte sectors starting at `lba` into `buf`.
/// Returns true on success.
pub fn read_sectors(lba: u64, count: u32, buf: &mut [u8]) -> bool {
    unsafe { crate::platform::qemu_virt::virtio_blk::disk_read_sectors(lba, count, buf) }
}

/// Write `count` 512-byte sectors starting at `lba` from `buf`.
/// Returns true on success.
pub fn write_sectors(lba: u64, count: u32, buf: &[u8]) -> bool {
    unsafe { crate::platform::qemu_virt::virtio_blk::disk_write_sectors(lba, count, buf) }
}

/// Total capacity in 512-byte sectors.
pub fn capacity() -> u64 {
    crate::platform::qemu_virt::virtio_blk::disk_capacity()
}

pub fn get_irq_id() -> u32 {
    crate::platform::qemu_virt::virtio_blk::disk_get_irq_id()
}

pub fn irq_handler() {
    unsafe { crate::platform::qemu_virt::virtio_blk::disk_irq_handler() };
}
