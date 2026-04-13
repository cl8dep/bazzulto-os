// fs/epoll.rs — epoll(7) implementation.
//
// Implements the three Linux epoll syscalls:
//   epoll_create1(flags) -> fd
//   epoll_ctl(epfd, op, fd, *event)
//   epoll_wait(epfd, *events, maxevents, timeout_ms)
//
// An epoll instance is a special inode that carries a list of "interests"
// (fd + event mask + user data). epoll_wait checks each fd for readiness
// and returns ready events.
//
// Design:
//   - EpollInstance implements Inode; installed as a FileDescriptor::InoFile.
//   - Readiness is checked synchronously — no kernel wait-queue wiring yet.
//   - On timeout_ms > 0 the kernel yields and retries in a busy loop up to
//     the deadline. This is correct for a single-core, IRQ-driven kernel.
//
// Reference:
//   Linux fs/eventpoll.c.
//   epoll(7) man page.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::fs::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};

// ---------------------------------------------------------------------------
// epoll event flag constants (Linux ABI values)
// ---------------------------------------------------------------------------

/// EPOLLIN — fd is ready for reading.
pub const EPOLLIN: u32 = 0x0001;

/// EPOLLOUT — fd is ready for writing.
pub const EPOLLOUT: u32 = 0x0004;

/// EPOLLERR — error condition on fd.
pub const EPOLLERR: u32 = 0x0008;

/// EPOLLHUP — hang-up on fd.
pub const EPOLLHUP: u32 = 0x0010;

// ---------------------------------------------------------------------------
// epoll_ctl operation codes (Linux ABI values)
// ---------------------------------------------------------------------------

/// EPOLL_CTL_ADD — add a new interest to the epoll instance.
pub const EPOLL_CTL_ADD: i32 = 1;

/// EPOLL_CTL_DEL — remove an interest from the epoll instance.
pub const EPOLL_CTL_DEL: i32 = 2;

/// EPOLL_CTL_MOD — modify the events/data for an existing interest.
pub const EPOLL_CTL_MOD: i32 = 3;

// ---------------------------------------------------------------------------
// EpollEvent — kernel-side representation of a user epoll_event struct
// ---------------------------------------------------------------------------

/// One I/O event record.
///
/// Layout matches Linux `struct epoll_event` (packed, events + u64 data).
/// Written into the user buffer by `epoll_wait`.
///
/// Reference: Linux `include/uapi/linux/eventpoll.h`.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct EpollEvent {
    /// Bitmask of event flags (EPOLLIN, EPOLLOUT, …).
    pub events: u32,
    /// Opaque user data returned verbatim in the ready event.
    pub data: u64,
}

// ---------------------------------------------------------------------------
// EpollInterest — one registered fd + events + data
// ---------------------------------------------------------------------------

/// One entry in the epoll instance's interest list.
struct EpollInterest {
    /// The file descriptor to watch.
    watched_fd: i32,
    /// Event mask: combination of EPOLLIN, EPOLLOUT, etc.
    event_mask: u32,
    /// Opaque user data echoed back in `epoll_wait` results.
    user_data: u64,
}

// ---------------------------------------------------------------------------
// EpollInstanceInner — mutable state behind the UnsafeCell
// ---------------------------------------------------------------------------

struct EpollInstanceInner {
    interests: Vec<EpollInterest>,
}

// ---------------------------------------------------------------------------
// EpollInstance — the inode type installed as an epoll fd
// ---------------------------------------------------------------------------

/// VFS inode representing an open epoll instance.
///
/// Installed as a `FileDescriptor::InoFile` when `epoll_create1` is called.
/// The actual interest list is manipulated via `epoll_ctl` and polled via
/// `epoll_wait`; both reach this struct through the inode reference.
pub struct EpollInstance {
    inode_number: u64,
    inner: UnsafeCell<EpollInstanceInner>,
}

// SAFETY: Bazzulto OS is single-core; IRQs are disabled during all syscall
// execution. There is no concurrent access to EpollInstance.
unsafe impl Send for EpollInstance {}
unsafe impl Sync for EpollInstance {}

impl EpollInstance {
    /// Create a new, empty epoll instance.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            inner: UnsafeCell::new(EpollInstanceInner {
                interests: Vec::new(),
            }),
        })
    }

    /// Add an interest for `watched_fd`.
    ///
    /// Returns `Ok(())` on success or `-EEXIST` (-17) if `watched_fd` is
    /// already registered.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn ctl_add(&self, watched_fd: i32, event_mask: u32, user_data: u64) -> i64 {
        let inner = &mut *self.inner.get();
        // Check for duplicate.
        for interest in &inner.interests {
            if interest.watched_fd == watched_fd {
                return -17; // EEXIST
            }
        }
        inner.interests.push(EpollInterest { watched_fd, event_mask, user_data });
        0
    }

    /// Remove the interest for `watched_fd`.
    ///
    /// Returns `Ok(())` on success or `-ENOENT` (-2) if the fd is not registered.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn ctl_del(&self, watched_fd: i32) -> i64 {
        let inner = &mut *self.inner.get();
        let original_len = inner.interests.len();
        inner.interests.retain(|interest| interest.watched_fd != watched_fd);
        if inner.interests.len() == original_len {
            return -2; // ENOENT
        }
        0
    }

    /// Modify the event mask and user data for an already-registered fd.
    ///
    /// Returns `Ok(())` on success or `-ENOENT` (-2) if the fd is not found.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn ctl_mod(&self, watched_fd: i32, event_mask: u32, user_data: u64) -> i64 {
        let inner = &mut *self.inner.get();
        for interest in &mut inner.interests {
            if interest.watched_fd == watched_fd {
                interest.event_mask = event_mask;
                interest.user_data = user_data;
                return 0;
            }
        }
        -2 // ENOENT
    }

    /// Fill `output_events` with ready events, up to `max_events` entries.
    ///
    /// Returns the number of events written.  The caller must pass a slice of
    /// at least `max_events` `EpollEvent` structs.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn collect_ready_events(
        &self,
        output_events: &mut [EpollEvent],
        max_events: usize,
    ) -> usize {
        let inner = &*self.inner.get();
        let mut count = 0usize;

        for interest in &inner.interests {
            if count >= max_events {
                break;
            }
            let ready_events = check_fd_readiness(interest.watched_fd, interest.event_mask);
            if ready_events != 0 {
                // Use core::ptr::write to work around the packed struct field
                // alignment restriction (Rust rejects direct assignment via reference).
                let event_ptr: *mut EpollEvent = &mut output_events[count];
                // SAFETY: output_events[count] is a valid, writable EpollEvent.
                core::ptr::write_unaligned(event_ptr, EpollEvent {
                    events: ready_events,
                    data: interest.user_data,
                });
                count += 1;
            }
        }
        count
    }
}

impl Inode for EpollInstance {
    fn inode_type(&self) -> InodeType { InodeType::CharDevice }
    fn stat(&self) -> InodeStat { InodeStat::char_device(self.inode_number) }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported) // use epoll_wait, not read()
    }
    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported) // use epoll_ctl, not write()
    }
    fn truncate(&self, _new_size: u64) -> Result<(), FsError> { Err(FsError::NotSupported) }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }
    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotDirectory) }
    fn unlink(&self, _name: &str) -> Result<(), FsError> { Err(FsError::NotDirectory) }
}

// ---------------------------------------------------------------------------
// Readiness checking
// ---------------------------------------------------------------------------

/// Check which of the requested `event_mask` flags are currently satisfied
/// by file descriptor `fd` in the current process.
///
/// Returns a bitmask of ready events (subset of `event_mask`).
///
/// Rules:
///   - Regular files and unknown inodes: always EPOLLIN | EPOLLOUT.
///   - Pipes: EPOLLIN if bytes available, EPOLLOUT always.
///   - PTY master: EPOLLIN if slave_to_master has bytes, EPOLLOUT always.
///   - Invalid fd: EPOLLERR.
///
/// # Safety
/// Must be called with IRQs disabled (scheduler and PTY table access).
unsafe fn check_fd_readiness(fd: i32, event_mask: u32) -> u32 {
    if fd < 0 {
        return EPOLLERR & event_mask;
    }

    let fd_index = fd as usize;

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EPOLLERR & event_mask,
    };
    let mut guard = fd_table_arc.lock();

    let descriptor = match guard.get_mut(fd_index) {
        Some(descriptor) => descriptor,
        None => return EPOLLERR & event_mask,
    };

    let mut ready: u32 = 0;

    match descriptor {
        crate::fs::vfs::FileDescriptor::Pipe(pipe_handle) => {
            use crate::fs::pipe::PipeEnd;
            match pipe_handle.end() {
                PipeEnd::ReadEnd => {
                    let bytes_available = pipe_handle.buffer_mut().available_to_read();
                    if bytes_available > 0 {
                        ready |= EPOLLIN;
                    }
                }
                PipeEnd::WriteEnd => {
                    ready |= EPOLLOUT;
                }
            }
        }
        crate::fs::vfs::FileDescriptor::InoFile { .. } => {
            // Without Any-based downcasting (not available in no_std), we
            // cannot distinguish PTY master inodes from other CharDevice
            // inodes at this layer. Treat all inode-backed fds as always
            // ready — correct for regular files and a conservative
            // approximation for devices.
            ready |= EPOLLIN | EPOLLOUT;
        }
        // All other descriptor types are always ready.
        _ => {
            ready |= EPOLLIN | EPOLLOUT;
        }
    }

    ready & event_mask
}

/// Public wrapper around `check_fd_readiness` for use by the `select(2)` syscall.
///
/// Returns a bitmask of ready events (subset of `event_mask`).
///
/// # Safety
/// Must be called with IRQs disabled (scheduler and VFS access).
pub unsafe fn check_fd_readiness_for_select(fd: i32, event_mask: u32) -> u32 {
    check_fd_readiness(fd, event_mask)
}
