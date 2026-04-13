// fs/devfs.rs — Device filesystem (/dev).
//
// Provides the standard character devices:
//   /dev/null    — reads return EOF; writes are discarded.
//   /dev/zero    — reads return zero bytes; writes are discarded.
//   /dev/urandom — reads return pseudo-random bytes from the kernel entropy pool.
//   /dev/tty     — reads/writes go to the process's controlling terminal.
//
// Reference: Linux drivers/char/mem.c.

extern crate alloc;

use alloc::sync::Arc;
use alloc::string::ToString;

use super::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};
use super::tmpfs::TmpfsDir;

// ---------------------------------------------------------------------------
// DevNull
// ---------------------------------------------------------------------------

pub struct DevNull { inode_number: u64 }
unsafe impl Send for DevNull {}
unsafe impl Sync for DevNull {}

impl DevNull {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number() })
    }
}

impl Inode for DevNull {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Ok(0) // EOF
    }
    fn write_at(&self, _offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        Ok(buf.len()) // discard
    }
    fn truncate(&self, _size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// DevZero
// ---------------------------------------------------------------------------

pub struct DevZero { inode_number: u64 }
unsafe impl Send for DevZero {}
unsafe impl Sync for DevZero {}

impl DevZero {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number() })
    }
}

impl Inode for DevZero {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        for byte in buf.iter_mut() { *byte = 0; }
        Ok(buf.len())
    }
    fn write_at(&self, _offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        Ok(buf.len())
    }
    fn truncate(&self, _size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// DevUrandom — LFSR-based pseudo-random device
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicU64, Ordering};

/// Entropy pool seed (populated in `devfs_init` from hardware entropy).
static URANDOM_STATE: AtomicU64 = AtomicU64::new(0xDEAD_BEEF_CAFE_1234);

/// Advance the Xorshift64 PRNG and return the next value.
fn urandom_next() -> u64 {
    let mut state = URANDOM_STATE.load(Ordering::Relaxed);
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    URANDOM_STATE.store(state, Ordering::Relaxed);
    state
}

pub struct DevUrandom { inode_number: u64 }
unsafe impl Send for DevUrandom {}
unsafe impl Sync for DevUrandom {}

impl DevUrandom {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number() })
    }
}

impl Inode for DevUrandom {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let mut index = 0;
        while index < buf.len() {
            let word = urandom_next();
            let bytes = word.to_le_bytes();
            let remaining = buf.len() - index;
            let to_copy = remaining.min(8);
            buf[index..index + to_copy].copy_from_slice(&bytes[..to_copy]);
            index += to_copy;
        }
        Ok(buf.len())
    }
    fn write_at(&self, _offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        // Accept entropy contributions from userspace.
        if !buf.is_empty() {
            let mut word = 0u64;
            for &byte in buf.iter().take(8) {
                word = word.wrapping_shl(8) | byte as u64;
            }
            let current = URANDOM_STATE.load(Ordering::Relaxed);
            URANDOM_STATE.store(current ^ word, Ordering::Relaxed);
        }
        Ok(buf.len())
    }
    fn truncate(&self, _size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// DevTty — process's controlling terminal
// ---------------------------------------------------------------------------

pub struct DevTty { inode_number: u64 }
unsafe impl Send for DevTty {}
unsafe impl Sync for DevTty {}

impl DevTty {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number() })
    }
}

impl Inode for DevTty {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let bytes_read = unsafe { crate::drivers::tty::tty_read_bytes(buf) };
        Ok(bytes_read)
    }
    fn write_at(&self, _offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        for &byte in buf {
            unsafe {
                crate::drivers::uart::putc(byte);
                crate::drivers::console::print_char(byte as char);
            }
        }
        Ok(buf.len())
    }
    fn truncate(&self, _size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// devfs_create — build the /dev directory tree
// ---------------------------------------------------------------------------

/// Build and return the /dev directory inode.
///
/// Populates:
///   /dev/null, /dev/zero, /dev/urandom, /dev/tty
///
/// Called from `vfs_init()` in `mount.rs` during kernel boot.
pub fn devfs_create() -> Arc<TmpfsDir> {
    let dev_dir = TmpfsDir::new();
    dev_dir.insert("null",    DevNull::new());
    dev_dir.insert("zero",    DevZero::new());
    dev_dir.insert("urandom", DevUrandom::new());
    dev_dir.insert("tty",     DevTty::new());
    dev_dir
}

/// Seed the urandom device with hardware entropy.
///
/// Called from `vfs_init()` after the timer is available.
pub fn devfs_seed_entropy(seed: u64) {
    let current = URANDOM_STATE.load(Ordering::Relaxed);
    URANDOM_STATE.store(current ^ seed, Ordering::Relaxed);
}
