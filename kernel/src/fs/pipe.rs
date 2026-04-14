// fs/pipe.rs — Kernel pipe: 64 KiB ring buffer with manual reference counting.
//
// Design choices (no Arc available without alloc feature flags issues):
//   - PipeBuffer is heap-allocated as a Box and its address is wrapped in a
//     RawPipe pointer together with a reference count maintained in the struct.
//   - The scheduler is called to block/unblock readers and writers.
//   - All access must be from a context with IRQs disabled (single-core invariant).
//
// Reference: POSIX.1-2017 §2.7 (pipe semantics).

extern crate alloc;

use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

// POSIX mandates at least 512 bytes; Linux uses 64 KiB as the default pipe
// capacity since kernel 2.6.11.  We match Linux's default.
pub const PIPE_BUFFER_CAPACITY: usize = 64 * 1024;

/// Ring buffer at the heart of a pipe.
///
/// The buffer is heap-allocated and addressed through `PipeHandle`.
/// All fields are modified only with IRQs disabled (single-core).
pub struct PipeBuffer {
    data: [u8; PIPE_BUFFER_CAPACITY],
    /// Index of the next byte to read.
    read_head: usize,
    /// Number of bytes currently in the buffer.
    length: usize,
    /// Number of write-end handles still alive.
    write_end_count: AtomicUsize,
    /// Number of read-end handles still alive.
    read_end_count: AtomicUsize,
    /// PID of a reader blocked waiting for data (None if none).
    blocked_reader: UnsafeCell<Option<crate::process::Pid>>,
    /// PID of a writer blocked waiting for space (None if none).
    blocked_writer: UnsafeCell<Option<crate::process::Pid>>,
}

// SAFETY: single-core; access serialised by IRQ disabling.
unsafe impl Sync for PipeBuffer {}
unsafe impl Send for PipeBuffer {}

impl PipeBuffer {
    fn new() -> Box<Self> {
        Box::new(Self {
            data: [0u8; PIPE_BUFFER_CAPACITY],
            read_head: 0,
            length: 0,
            write_end_count: AtomicUsize::new(1),
            read_end_count: AtomicUsize::new(1),
            blocked_reader: UnsafeCell::new(None),
            blocked_writer: UnsafeCell::new(None),
        })
    }

    pub fn available_to_read(&self) -> usize {
        self.length
    }

    pub fn available_to_write(&self) -> usize {
        PIPE_BUFFER_CAPACITY - self.length
    }

    /// True if all write ends have been closed (EOF on read).
    pub fn is_write_closed(&self) -> bool {
        self.write_end_count.load(Ordering::Acquire) == 0
    }

    /// True if all read ends have been closed (SIGPIPE on write).
    pub fn is_read_closed(&self) -> bool {
        self.read_end_count.load(Ordering::Acquire) == 0
    }

    /// Write up to `buf.len()` bytes into the ring buffer.
    /// Returns the number of bytes written.
    pub fn write_bytes(&mut self, source_buffer: &[u8]) -> usize {
        let space = self.available_to_write();
        let to_write = source_buffer.len().min(space);
        if to_write == 0 {
            return 0;
        }
        let write_head = (self.read_head + self.length) % PIPE_BUFFER_CAPACITY;
        let first_chunk = to_write.min(PIPE_BUFFER_CAPACITY - write_head);
        self.data[write_head..write_head + first_chunk]
            .copy_from_slice(&source_buffer[..first_chunk]);
        if first_chunk < to_write {
            let second_chunk = to_write - first_chunk;
            self.data[..second_chunk].copy_from_slice(&source_buffer[first_chunk..to_write]);
        }
        self.length += to_write;
        to_write
    }

    /// Read up to `destination_buffer.len()` bytes from the ring buffer.
    /// Returns the number of bytes read.
    pub fn read_bytes(&mut self, destination_buffer: &mut [u8]) -> usize {
        let available = self.available_to_read();
        let to_read = destination_buffer.len().min(available);
        if to_read == 0 {
            return 0;
        }
        let first_chunk = to_read.min(PIPE_BUFFER_CAPACITY - self.read_head);
        destination_buffer[..first_chunk]
            .copy_from_slice(&self.data[self.read_head..self.read_head + first_chunk]);
        if first_chunk < to_read {
            let second_chunk = to_read - first_chunk;
            destination_buffer[first_chunk..to_read].copy_from_slice(&self.data[..second_chunk]);
        }
        self.read_head = (self.read_head + to_read) % PIPE_BUFFER_CAPACITY;
        self.length -= to_read;
        to_read
    }

    pub fn set_blocked_reader(&self, pid: Option<crate::process::Pid>) {
        unsafe { *self.blocked_reader.get() = pid };
    }

    pub fn blocked_reader(&self) -> Option<crate::process::Pid> {
        unsafe { *self.blocked_reader.get() }
    }

    /// Wake the blocked reader (if any) after data has been written to the buffer.
    ///
    /// Used by the TTY echo path to unblock bzdisplayd without going through
    /// the full `pipe_write_blocking` helper.  Safe to call with IRQs disabled.
    pub fn wake_blocked_reader(&self) {
        if let Some(reader_pid) = self.blocked_reader() {
            self.set_blocked_reader(None);
            unsafe {
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(reader_pid);
                });
            }
        }
    }

    pub fn set_blocked_writer(&self, pid: Option<crate::process::Pid>) {
        unsafe { *self.blocked_writer.get() = pid };
    }

    pub fn blocked_writer(&self) -> Option<crate::process::Pid> {
        unsafe { *self.blocked_writer.get() }
    }
}

// ---------------------------------------------------------------------------
// PipeHandle — a reference to one end (read or write) of a pipe.
// ---------------------------------------------------------------------------

/// Which end of a pipe this handle refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PipeEnd {
    ReadEnd,
    WriteEnd,
}

/// A cloneable handle to one end of a kernel pipe.
///
/// Dropping a handle decrements the appropriate reference count on the buffer.
/// When both read-end count and write-end count reach zero the buffer is freed.
pub struct PipeHandle {
    /// Raw pointer to the heap-allocated PipeBuffer.
    ///
    /// SAFETY: valid as long as either end count > 0.
    raw: *mut PipeBuffer,
    end: PipeEnd,
}

// SAFETY: single-core; send/sync enforced by IRQ disabling.
unsafe impl Send for PipeHandle {}
unsafe impl Sync for PipeHandle {}

impl PipeHandle {
    pub fn end(&self) -> PipeEnd {
        self.end
    }

    pub fn buffer_mut(&self) -> &mut PipeBuffer {
        unsafe { &mut *self.raw }
    }

    /// Clone the read end (increments read_end_count).
    pub fn clone_read_end(read_handle: &PipeHandle) -> PipeHandle {
        assert_eq!(read_handle.end, PipeEnd::ReadEnd);
        let buf = unsafe { &*read_handle.raw };
        buf.read_end_count.fetch_add(1, Ordering::AcqRel);
        PipeHandle { raw: read_handle.raw, end: PipeEnd::ReadEnd }
    }

    /// Clone the write end (increments write_end_count).
    pub fn clone_write_end(write_handle: &PipeHandle) -> PipeHandle {
        assert_eq!(write_handle.end, PipeEnd::WriteEnd);
        let buf = unsafe { &*write_handle.raw };
        buf.write_end_count.fetch_add(1, Ordering::AcqRel);
        PipeHandle { raw: write_handle.raw, end: PipeEnd::WriteEnd }
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        let buf = unsafe { &*self.raw };
        match self.end {
            PipeEnd::ReadEnd => {
                let previous = buf.read_end_count.fetch_sub(1, Ordering::AcqRel);
                // If a writer is blocked on a full buffer, wake it so it can
                // get EPIPE / detect the closed read end.
                if previous == 1 {
                    if let Some(writer_pid) = buf.blocked_writer() {
                        unsafe {
                            crate::scheduler::with_scheduler(|scheduler| {
                                scheduler.unblock(writer_pid);
                            });
                        }
                    }
                }
            }
            PipeEnd::WriteEnd => {
                let previous = buf.write_end_count.fetch_sub(1, Ordering::AcqRel);
                // If a reader is blocked waiting for data, wake it to get EOF.
                if previous == 1 {
                    if let Some(reader_pid) = buf.blocked_reader() {
                        unsafe {
                            crate::scheduler::with_scheduler(|scheduler| {
                                scheduler.unblock(reader_pid);
                            });
                        }
                    }
                }
            }
        }

        // Free the buffer only when BOTH ends are fully closed.
        let write_count = buf.write_end_count.load(Ordering::Acquire);
        let read_count = buf.read_end_count.load(Ordering::Acquire);
        if write_count == 0 && read_count == 0 {
            // Reconstruct the Box so Rust drops it properly.
            unsafe { drop(Box::from_raw(self.raw)) };
        }
    }
}

// ---------------------------------------------------------------------------
// Public constructor
// ---------------------------------------------------------------------------

/// Create a new pipe and return (read_handle, write_handle).
pub fn pipe_create() -> (PipeHandle, PipeHandle) {
    let buffer = Box::into_raw(PipeBuffer::new());
    // Both ends start with ref-count = 1 (set in PipeBuffer::new).
    let read_handle = PipeHandle { raw: buffer, end: PipeEnd::ReadEnd };
    let write_handle = PipeHandle { raw: buffer, end: PipeEnd::WriteEnd };
    (read_handle, write_handle)
}

// ---------------------------------------------------------------------------
// Blocking I/O helpers — called from syscall layer with IRQs disabled.
// ---------------------------------------------------------------------------

/// Read up to `destination.len()` bytes from the pipe, blocking if empty.
///
/// Returns the number of bytes read, or 0 on EOF (all write ends closed).
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pipe_read_blocking(
    handle: &PipeHandle,
    destination: &mut [u8],
) -> usize {
    loop {
        let buf = handle.buffer_mut();
        let avail = buf.available_to_read();
        if avail > 0 {
            let bytes_read = buf.read_bytes(destination);
            // Wake a blocked writer that was waiting for space.
            if let Some(writer_pid) = buf.blocked_writer() {
                buf.set_blocked_writer(None);
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(writer_pid);
                });
            }
            return bytes_read;
        }
        // Fast-path EOF: safe here because the TOCTOU-safe check below covers
        // the race window between this point and set_blocked_reader.
        if buf.is_write_closed() {
            return 0; // Buffer empty and all write ends closed → EOF.
        }
        // TOCTOU fix: set blocked_reader BEFORE the final is_write_closed()
        // check so that a concurrent close will always see blocked_reader set
        // and will wake us.  If the close already happened between the fast-path
        // check above and set_blocked_reader, we detect it here and skip
        // blocking; the loop then re-checks available_to_read() (to drain any
        // data the writer deposited in that window) and the fast-path EOF check.
        crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            handle.buffer_mut().set_blocked_reader(Some(current_pid));
            if handle.buffer_mut().is_write_closed() {
                // Write end closed in the race window.  Clear blocked_reader and
                // let the loop continue rather than blocking.
                handle.buffer_mut().set_blocked_reader(None);
            } else {
                // Safe to block: any future close will see blocked_reader set.
                scheduler.block_current();
            }
            // After unblocking: IRQs are disabled again by the scheduler.
        });
    }
}

/// Result of `pipe_write_blocking`.
///
/// Distinguishes a successful write from a broken-pipe condition so that
/// the syscall layer can send `SIGPIPE` and return `EPIPE` as POSIX requires.
pub enum PipeWriteResult {
    /// Bytes were written successfully.  Payload is the total byte count.
    Written(usize),
    /// The read end of the pipe is closed.  POSIX requires SIGPIPE + EPIPE.
    /// Payload is the number of bytes written *before* the broken-pipe was
    /// detected (may be > 0 for partial writes on large buffers).
    BrokenPipe(usize),
}

/// Write all of `source` into the pipe, blocking if the buffer is full.
///
/// Returns `PipeWriteResult::Written(n)` on success or
/// `PipeWriteResult::BrokenPipe(n)` if all read ends are closed.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn pipe_write_blocking(
    handle: &PipeHandle,
    source: &[u8],
) -> PipeWriteResult {
    let mut total_written = 0usize;
    let mut remaining = source;

    loop {
        if remaining.is_empty() {
            return PipeWriteResult::Written(total_written);
        }
        let buf = handle.buffer_mut();
        if buf.is_read_closed() {
            // POSIX.1-2017 write(2): if all read ends are closed, the write
            // shall fail with EPIPE and SIGPIPE shall be generated.
            return PipeWriteResult::BrokenPipe(total_written);
        }
        let written = buf.write_bytes(remaining);
        if written > 0 {
            total_written += written;
            remaining = &remaining[written..];
            // Wake a blocked reader.
            if let Some(reader_pid) = buf.blocked_reader() {
                buf.set_blocked_reader(None);
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(reader_pid);
                });
            }
        } else {
            // Buffer full: block until a reader drains some data.
            crate::scheduler::with_scheduler(|scheduler| {
                let current_pid = scheduler.current_pid();
                handle.buffer_mut().set_blocked_writer(Some(current_pid));
                scheduler.block_current();
            });
        }
    }
}
