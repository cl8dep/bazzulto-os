// drivers/pty.rs — Pseudo-terminal (PTY) pair driver.
//
// A PTY is a bidirectional pair of character devices:
//   - The master side is opened via /dev/ptmx.  The process on the master side
//     drives the terminal (e.g., a terminal emulator or SSH daemon).
//   - The slave side (/dev/pts/N) looks like a real terminal to the process
//     running inside it (e.g., a shell).
//
// Data flow:
//   master write → master_to_slave buffer → slave read
//   slave  write → slave_to_master buffer → master read
//
// Reference:
//   POSIX.1-2017 §11 (General Terminal Interface, pts(4)).
//   Linux Documentation/driver-api/tty/pty.rst.

extern crate alloc;

use alloc::sync::Arc;
use core::cell::UnsafeCell;

use crate::fs::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};
use crate::drivers::tty::Termios;

// ---------------------------------------------------------------------------
// Capacity constants
// ---------------------------------------------------------------------------

/// Maximum number of simultaneously allocated PTY pairs.
pub const PTY_MAX: usize = 16;

/// Ring buffer capacity per direction (master→slave and slave→master).
const PTY_RING_BUFFER_SIZE: usize = 4096;

// ---------------------------------------------------------------------------
// PtyRingBuffer — lock-free ring for single-core use
// ---------------------------------------------------------------------------

struct PtyRingBuffer {
    data: [u8; PTY_RING_BUFFER_SIZE],
    read_position: usize,
    write_position: usize,
    count: usize,
}

impl PtyRingBuffer {
    const fn new() -> Self {
        Self {
            data: [0u8; PTY_RING_BUFFER_SIZE],
            read_position: 0,
            write_position: 0,
            count: 0,
        }
    }

    /// Copy bytes from this buffer into `destination`.
    ///
    /// Returns the number of bytes actually read (may be less than
    /// `destination.len()` if fewer bytes are available).
    fn read(&mut self, destination: &mut [u8]) -> usize {
        let to_read = destination.len().min(self.count);
        for index in 0..to_read {
            destination[index] = self.data[self.read_position];
            self.read_position = (self.read_position + 1) % PTY_RING_BUFFER_SIZE;
        }
        self.count -= to_read;
        to_read
    }

    /// Copy bytes from `source` into this buffer.
    ///
    /// Returns the number of bytes actually written (may be less than
    /// `source.len()` if the buffer is full).
    fn write(&mut self, source: &[u8]) -> usize {
        let free_space = PTY_RING_BUFFER_SIZE - self.count;
        let to_write = source.len().min(free_space);
        for index in 0..to_write {
            self.data[self.write_position] = source[index];
            self.write_position = (self.write_position + 1) % PTY_RING_BUFFER_SIZE;
        }
        self.count += to_write;
        to_write
    }

    /// Number of bytes available to read.
    fn available(&self) -> usize {
        self.count
    }
}

// ---------------------------------------------------------------------------
// PtyPair — one allocated PTY pair (master + slave state)
// ---------------------------------------------------------------------------

struct PtyPair {
    /// True when this slot is in use.
    allocated: bool,
    /// Bytes written by the master; read by the slave.
    master_to_slave: PtyRingBuffer,
    /// Bytes written by the slave; read by the master.
    slave_to_master: PtyRingBuffer,
    /// Termios settings maintained on the slave side.
    termios: Termios,
    /// Terminal window height in character cells.
    window_rows: u16,
    /// Terminal window width in character cells.
    window_cols: u16,
}

impl PtyPair {
    const fn new() -> Self {
        Self {
            allocated: false,
            master_to_slave: PtyRingBuffer::new(),
            slave_to_master: PtyRingBuffer::new(),
            termios: Termios::cooked_defaults(),
            window_rows: 24,
            window_cols: 80,
        }
    }
}

// ---------------------------------------------------------------------------
// Global PTY table
// ---------------------------------------------------------------------------

struct PtyTable(UnsafeCell<[PtyPair; PTY_MAX]>);

// SAFETY: Bazzulto OS is single-core; IRQs are disabled during all kernel
// operations that touch the PTY table.
unsafe impl Sync for PtyTable {}

/// Global table of all PTY pairs.
///
/// Index 0–(PTY_MAX-1) correspond to /dev/pts/0 through /dev/pts/(PTY_MAX-1).
static PTY_TABLE: PtyTable = PtyTable(UnsafeCell::new([
    PtyPair::new(), PtyPair::new(), PtyPair::new(), PtyPair::new(),
    PtyPair::new(), PtyPair::new(), PtyPair::new(), PtyPair::new(),
    PtyPair::new(), PtyPair::new(), PtyPair::new(), PtyPair::new(),
    PtyPair::new(), PtyPair::new(), PtyPair::new(), PtyPair::new(),
]));

/// Access the global PTY table mutably.
///
/// # Safety
/// Caller must ensure IRQs are disabled (single-core invariant).
unsafe fn with_pty_table<F, R>(function: F) -> R
where
    F: FnOnce(&mut [PtyPair; PTY_MAX]) -> R,
{
    function(&mut *PTY_TABLE.0.get())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Allocate a new PTY pair.
///
/// Returns the PTY index (0–PTY_MAX-1) on success, or `None` if the table
/// is full.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pty_allocate() -> Option<usize> {
    with_pty_table(|table| {
        for (index, pair) in table.iter_mut().enumerate() {
            if !pair.allocated {
                pair.allocated = true;
                pair.master_to_slave = PtyRingBuffer::new();
                pair.slave_to_master = PtyRingBuffer::new();
                pair.termios = Termios::cooked_defaults();
                pair.window_rows = 24;
                pair.window_cols = 80;
                return Some(index);
            }
        }
        None
    })
}

/// Release a PTY pair back to the pool.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pty_free(index: usize) {
    with_pty_table(|table| {
        if index < PTY_MAX {
            table[index].allocated = false;
        }
    });
}

/// Return a master-side inode for the given PTY index.
pub fn pty_master_inode(index: usize) -> Arc<dyn Inode> {
    Arc::new(PtyMasterInode {
        inode_number: alloc_inode_number(),
        pty_index: index,
    })
}

/// Return a slave-side inode for the given PTY index.
pub fn pty_slave_inode(index: usize) -> Arc<dyn Inode> {
    Arc::new(PtySlaveInode {
        inode_number: alloc_inode_number(),
        pty_index: index,
    })
}

/// Return the number of bytes available to read from the slave-to-master buffer.
///
/// Used by epoll to determine whether the master fd is readable.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pty_master_readable_count(index: usize) -> usize {
    with_pty_table(|table| {
        if index < PTY_MAX && table[index].allocated {
            table[index].slave_to_master.available()
        } else {
            0
        }
    })
}

/// Read the current window size for the given PTY.
///
/// Returns `(rows, cols)` or `(0, 0)` if the index is invalid.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pty_get_window_size(index: usize) -> (u16, u16) {
    with_pty_table(|table| {
        if index < PTY_MAX && table[index].allocated {
            (table[index].window_rows, table[index].window_cols)
        } else {
            (0, 0)
        }
    })
}

/// Set the window size for the given PTY.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pty_set_window_size(index: usize, rows: u16, cols: u16) {
    with_pty_table(|table| {
        if index < PTY_MAX && table[index].allocated {
            table[index].window_rows = rows;
            table[index].window_cols = cols;
        }
    });
}

// ---------------------------------------------------------------------------
// PtyMasterInode — master side of a PTY pair
// ---------------------------------------------------------------------------

/// Inode representing the master side of PTY pair N.
///
/// Opened when `sys_open("/dev/ptmx", …)` allocates a new PTY.
/// The master reads bytes written by the slave (slave_to_master direction)
/// and writes bytes to be delivered to the slave (master_to_slave direction).
pub struct PtyMasterInode {
    inode_number: u64,
    /// Index into the global PTY table.
    pub pty_index: usize,
}

unsafe impl Send for PtyMasterInode {}
unsafe impl Sync for PtyMasterInode {}

impl Inode for PtyMasterInode {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, destination: &mut [u8]) -> Result<usize, FsError> {
        // Master reads what the slave wrote (slave_to_master direction).
        // SAFETY: single-core, IRQs disabled at kernel entry.
        let bytes_read = unsafe {
            with_pty_table(|table| {
                if self.pty_index < PTY_MAX && table[self.pty_index].allocated {
                    table[self.pty_index].slave_to_master.read(destination)
                } else {
                    0
                }
            })
        };
        Ok(bytes_read)
    }

    fn write_at(&self, _offset: u64, source: &[u8]) -> Result<usize, FsError> {
        // Master writes to the master_to_slave buffer; slave will read this.
        // SAFETY: single-core, IRQs disabled at kernel entry.
        let bytes_written = unsafe {
            with_pty_table(|table| {
                if self.pty_index < PTY_MAX && table[self.pty_index].allocated {
                    table[self.pty_index].master_to_slave.write(source)
                } else {
                    0
                }
            })
        };
        Ok(bytes_written)
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// PtySlaveInode — slave side of a PTY pair
// ---------------------------------------------------------------------------

/// Inode representing the slave side of PTY pair N (/dev/pts/N).
///
/// The slave reads bytes written by the master (master_to_slave direction)
/// and writes bytes to be read by the master (slave_to_master direction).
pub struct PtySlaveInode {
    inode_number: u64,
    /// Index into the global PTY table.
    pub pty_index: usize,
}

unsafe impl Send for PtySlaveInode {}
unsafe impl Sync for PtySlaveInode {}

impl Inode for PtySlaveInode {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, destination: &mut [u8]) -> Result<usize, FsError> {
        // Slave reads what the master wrote (master_to_slave direction).
        // SAFETY: single-core, IRQs disabled at kernel entry.
        let bytes_read = unsafe {
            with_pty_table(|table| {
                if self.pty_index < PTY_MAX && table[self.pty_index].allocated {
                    table[self.pty_index].master_to_slave.read(destination)
                } else {
                    0
                }
            })
        };
        Ok(bytes_read)
    }

    fn write_at(&self, _offset: u64, source: &[u8]) -> Result<usize, FsError> {
        // Slave writes to slave_to_master buffer; master will read this.
        // SAFETY: single-core, IRQs disabled at kernel entry.
        let bytes_written = unsafe {
            with_pty_table(|table| {
                if self.pty_index < PTY_MAX && table[self.pty_index].allocated {
                    table[self.pty_index].slave_to_master.write(source)
                } else {
                    0
                }
            })
        };
        Ok(bytes_written)
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> { Ok(()) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}
