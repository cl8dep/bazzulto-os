// fs/fifo.rs — FIFO (named pipe) inode.
//
// A FIFO inode wraps a shared ring buffer.  Multiple processes can open the
// same FIFO path; reads consume from the buffer, writes produce into it.
// Blocking semantics: read blocks if buffer empty and writers exist,
// write blocks if buffer full and readers exist.
//
// The shared buffer lives inside the FifoInode, which is itself held behind
// an Arc<dyn Inode> in the tmpfs directory.  Every open() call that resolves
// to this inode gets a clone of the same Arc, so all openers share the same
// ring buffer.
//
// Reference: POSIX.1-2017 §10 (Pipes and FIFOs).

extern crate alloc;

use core::cell::UnsafeCell;

use super::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};
use alloc::sync::Arc;

// ---------------------------------------------------------------------------
// Ring buffer capacity
// ---------------------------------------------------------------------------

/// Capacity of the FIFO ring buffer in bytes.
///
/// Matches the Linux default pipe capacity (since kernel 2.6.11).
/// POSIX mandates at least PIPE_BUF (512 bytes); we use 4 KiB as a
/// reasonable minimum for a named-pipe implementation.
pub const FIFO_BUFFER_CAPACITY: usize = 4096;

// ---------------------------------------------------------------------------
// FifoBuffer — ring buffer plus open-end counters
// ---------------------------------------------------------------------------

struct FifoBuffer {
    /// Byte storage for the ring buffer.
    data: [u8; FIFO_BUFFER_CAPACITY],
    /// Index of the next byte to read (always < FIFO_BUFFER_CAPACITY).
    read_position: usize,
    /// Number of bytes currently in the buffer.
    byte_count: usize,
    /// Number of file descriptors currently open for writing.
    ///
    /// When this reaches zero the read end sees EOF (no more writers).
    writer_count: usize,
    /// Number of file descriptors currently open for reading.
    ///
    /// When this reaches zero writers receive EPIPE.
    reader_count: usize,
    /// PID of a reader that is blocked waiting for data, if any.
    blocked_reader_pid: Option<crate::process::Pid>,
    /// PID of a writer that is blocked waiting for space, if any.
    blocked_writer_pid: Option<crate::process::Pid>,
}

impl FifoBuffer {
    const fn new() -> Self {
        Self {
            data: [0u8; FIFO_BUFFER_CAPACITY],
            read_position: 0,
            byte_count: 0,
            writer_count: 1,
            reader_count: 1,
            blocked_reader_pid: None,
            blocked_writer_pid: None,
        }
    }

    fn available_to_read(&self) -> usize {
        self.byte_count
    }

    fn available_to_write(&self) -> usize {
        FIFO_BUFFER_CAPACITY - self.byte_count
    }

    /// Copy up to `destination.len()` bytes out of the ring buffer.
    ///
    /// Returns the number of bytes copied.
    fn consume_bytes(&mut self, destination: &mut [u8]) -> usize {
        let to_read = destination.len().min(self.byte_count);
        if to_read == 0 {
            return 0;
        }
        let first_chunk = to_read.min(FIFO_BUFFER_CAPACITY - self.read_position);
        destination[..first_chunk]
            .copy_from_slice(&self.data[self.read_position..self.read_position + first_chunk]);
        if first_chunk < to_read {
            let second_chunk = to_read - first_chunk;
            destination[first_chunk..to_read].copy_from_slice(&self.data[..second_chunk]);
        }
        self.read_position = (self.read_position + to_read) % FIFO_BUFFER_CAPACITY;
        self.byte_count -= to_read;
        to_read
    }

    /// Copy up to `source.len()` bytes into the ring buffer.
    ///
    /// Returns the number of bytes copied (may be less than source.len() if
    /// the buffer is almost full).
    fn produce_bytes(&mut self, source: &[u8]) -> usize {
        let space = self.available_to_write();
        let to_write = source.len().min(space);
        if to_write == 0 {
            return 0;
        }
        let write_position = (self.read_position + self.byte_count) % FIFO_BUFFER_CAPACITY;
        let first_chunk = to_write.min(FIFO_BUFFER_CAPACITY - write_position);
        self.data[write_position..write_position + first_chunk]
            .copy_from_slice(&source[..first_chunk]);
        if first_chunk < to_write {
            let second_chunk = to_write - first_chunk;
            self.data[..second_chunk].copy_from_slice(&source[first_chunk..to_write]);
        }
        self.byte_count += to_write;
        to_write
    }
}

// ---------------------------------------------------------------------------
// FifoInode
// ---------------------------------------------------------------------------

/// Named-pipe inode.
///
/// Lives in the VFS as an `Arc<dyn Inode>`.  All openers share the same
/// `FifoBuffer` through the Arc, so reads and writes across processes
/// communicate through the same ring buffer.
///
/// Interior mutability via `UnsafeCell`.  Safe on single-core with IRQs off.
pub struct FifoInode {
    inode_number: u64,
    inner: UnsafeCell<FifoBuffer>,
}

// SAFETY: Bazzulto OS is single-core with IRQs disabled during all kernel
// operations.  There is never concurrent access from multiple hardware threads.
unsafe impl Send for FifoInode {}
unsafe impl Sync for FifoInode {}

impl FifoInode {
    /// Allocate a new FIFO inode with an empty ring buffer.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            inner: UnsafeCell::new(FifoBuffer::new()),
        })
    }

    /// Increment the reader open-count.
    ///
    /// Called by the open path when a process opens the FIFO for reading.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn increment_reader_count(&self) {
        let buffer = &mut *self.inner.get();
        buffer.reader_count += 1;
    }

    /// Increment the writer open-count.
    ///
    /// Called by the open path when a process opens the FIFO for writing.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn increment_writer_count(&self) {
        let buffer = &mut *self.inner.get();
        buffer.writer_count += 1;
    }

    /// Decrement the reader open-count and wake any blocked writer.
    ///
    /// Called when a read-end file descriptor is closed.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn decrement_reader_count(&self) {
        let buffer = &mut *self.inner.get();
        if buffer.reader_count > 0 {
            buffer.reader_count -= 1;
        }
        // If the last reader closed, wake a blocked writer so it can observe
        // EPIPE / BrokenPipe on its next write attempt.
        if buffer.reader_count == 0 {
            if let Some(writer_pid) = buffer.blocked_writer_pid.take() {
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(writer_pid);
                });
            }
        }
    }

    /// Decrement the writer open-count and wake any blocked reader.
    ///
    /// Called when a write-end file descriptor is closed.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn decrement_writer_count(&self) {
        let buffer = &mut *self.inner.get();
        if buffer.writer_count > 0 {
            buffer.writer_count -= 1;
        }
        // If the last writer closed, wake a blocked reader so it can observe
        // EOF on its next read attempt.
        if buffer.writer_count == 0 {
            if let Some(reader_pid) = buffer.blocked_reader_pid.take() {
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(reader_pid);
                });
            }
        }
    }
}

impl Inode for FifoInode {
    fn inode_type(&self) -> InodeType {
        InodeType::Fifo
    }

    fn stat(&self) -> InodeStat {
        // SAFETY: single-core, IRQs disabled at callsite.
        let buffer = unsafe { &*self.inner.get() };
        InodeStat {
            inode_number: self.inode_number,
            size: buffer.byte_count as u64,
            // st_mode for FIFO: 0o010666 (S_IFIFO = 0o010000 | 0o666).
            // Reference: POSIX.1-2017 sys/stat.h S_IFIFO.
            mode: 0o010666,
            nlinks: 1,
            uid: 0,
            gid: 0,
        }
    }

    /// Read up to `destination.len()` bytes from the FIFO ring buffer.
    ///
    /// The `offset` parameter is ignored; FIFOs are sequential.
    ///
    /// Blocking behaviour:
    ///   - If the buffer is empty and writers are still open: block the calling
    ///     process (set state to Blocked, store PID, let the scheduler run the
    ///     next ready process).
    ///   - If the buffer is empty and `writer_count == 0`: return Ok(0) (EOF).
    ///   - If data is available: consume up to buf.len() bytes and return Ok(n).
    ///
    /// # Safety
    /// Must be called with IRQs disabled (single-core invariant).
    fn read_at(&self, _offset: u64, destination: &mut [u8]) -> Result<usize, FsError> {
        loop {
            // SAFETY: single-core, IRQs disabled.
            let buffer = unsafe { &mut *self.inner.get() };

            if buffer.available_to_read() > 0 {
                let bytes_read = buffer.consume_bytes(destination);
                // Wake a writer that was blocked on a full buffer.
                if let Some(writer_pid) = buffer.blocked_writer_pid.take() {
                    unsafe {
                        crate::scheduler::with_scheduler(|scheduler| {
                            scheduler.unblock(writer_pid);
                        });
                    }
                }
                return Ok(bytes_read);
            }

            // Buffer is empty.
            if buffer.writer_count == 0 {
                return Ok(0); // EOF — all writers have closed.
            }

            // Block until a writer produces data.
            unsafe {
                crate::scheduler::with_scheduler(|scheduler| {
                    let current_pid = scheduler.current_pid();
                    // SAFETY: single-core, IRQs disabled.
                    let buf = &mut *self.inner.get();
                    buf.blocked_reader_pid = Some(current_pid);
                    // SAFETY: sets process state to Blocked and switches to next process.
                    unsafe { scheduler.block_current() };
                });
            }
            // Loop: after unblocking, check the buffer again.
        }
    }

    /// Write `source` bytes into the FIFO ring buffer.
    ///
    /// The `offset` parameter is ignored; FIFOs are sequential.
    ///
    /// Blocking behaviour:
    ///   - If `reader_count == 0`: return Err(FsError::BrokenPipe) (EPIPE).
    ///   - If the buffer is full: block until a reader drains space.
    ///   - Otherwise: write as many bytes as fit and return Ok(n).
    ///
    /// # Safety
    /// Must be called with IRQs disabled (single-core invariant).
    fn write_at(&self, _offset: u64, source: &[u8]) -> Result<usize, FsError> {
        let mut total_written = 0usize;
        let mut remaining = source;

        loop {
            if remaining.is_empty() {
                return Ok(total_written);
            }

            // SAFETY: single-core, IRQs disabled.
            let buffer = unsafe { &mut *self.inner.get() };

            if buffer.reader_count == 0 {
                return Err(FsError::BrokenPipe);
            }

            let written = buffer.produce_bytes(remaining);
            if written > 0 {
                total_written += written;
                remaining = &remaining[written..];
                // Wake a reader that was blocked waiting for data.
                if let Some(reader_pid) = buffer.blocked_reader_pid.take() {
                    unsafe {
                        crate::scheduler::with_scheduler(|scheduler| {
                            scheduler.unblock(reader_pid);
                        });
                    }
                }
            } else {
                // Buffer full — block until a reader drains some space.
                unsafe {
                    crate::scheduler::with_scheduler(|scheduler| {
                        let current_pid = scheduler.current_pid();
                        // SAFETY: single-core, IRQs disabled.
                        let buf = &mut *self.inner.get();
                        buf.blocked_writer_pid = Some(current_pid);
                        // SAFETY: sets process state to Blocked and switches to next process.
                        unsafe { scheduler.block_current() };
                    });
                }
            }
        }
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> {
        None
    }

    fn readdir(&self, _index: usize) -> Option<DirEntry> {
        None
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
