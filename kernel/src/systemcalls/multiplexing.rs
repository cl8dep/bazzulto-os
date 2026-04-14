// multiplexing.rs — I/O multiplexing syscall implementations.
//
// Syscalls: poll, select

use super::*;

// sys_poll — wait for events on file descriptors
//
// ABI: poll(fds, nfds, timeout_ms)
//
// struct pollfd (3 × u32 = 12 bytes each, but we use u64 layout for simplicity):
//   fds[i*2+0]: fd (lower 32 bits) | events (upper 32 bits)
//   fds[i*2+1]: revents (lower 32 bits, filled in by kernel)
//
// Events supported:
//   POLLIN  (0x0001) — data available to read
//   POLLOUT (0x0004) — space available to write
//
// Simplified implementation:
//   - Returns immediately with all requested events satisfied (always ready).
//   - Proper blocking poll requires an event queue (Fase 9).
// ---------------------------------------------------------------------------

const POLLIN:  u16 = 0x0001;
const POLLOUT: u16 = 0x0004;
const POLLERR: u16 = 0x0008;
const POLLHUP: u16 = 0x0010;
const POLLNVAL: u16 = 0x0020;

/// sys_poll(fds, nfds, timeout_ms) — wait for events on file descriptors.
///
/// `fds` points to an array of `nfds` Linux struct pollfd entries.
/// Linux struct pollfd layout (8 bytes):
///   offset 0: i32  fd
///   offset 4: u16  events   (requested events)
///   offset 6: u16  revents  (returned events — kernel fills this)
///
/// Reference: POSIX.1-2017 poll(2), Linux include/uapi/asm-generic/poll.h.
pub(super) unsafe fn sys_poll(fds_ptr: *mut u8, nfds: usize, timeout_ms: i32) -> i64 {
    // Linux struct pollfd is 8 bytes: i32 fd + u16 events + u16 revents.
    const POLLFD_SIZE: usize = 8;

    if nfds == 0 {
        // Zero FDs: sleep for timeout and return 0.
        if timeout_ms > 0 {
            let tick_interval_ns = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS * 1_000_000;
            let total_ns = (timeout_ms as u64) * 1_000_000;
            let ticks_to_sleep = total_ns / tick_interval_ns + 1;
            let wake_at = crate::platform::qemu_virt::timer::current_tick()
                .saturating_add(ticks_to_sleep);
            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process_mut() {
                    process.state = crate::process::ProcessState::Sleeping { wake_at_tick: wake_at };
                }
                scheduler.schedule();
            });
        }
        return 0;
    }

    if !validate_user_pointer(fds_ptr as u64, nfds * POLLFD_SIZE) {
        return EINVAL;
    }

    let mut ready_count: i64 = 0;

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let guard = fd_table_arc.lock();

    for index in 0..nfds {
        // Byte pointer to start of this pollfd entry (8 bytes each).
        let entry = fds_ptr.add(index * POLLFD_SIZE);

        // Read fd at offset 0 (i32, little-endian).
        let fd = i32::from_le_bytes([
            *entry,
            *entry.add(1),
            *entry.add(2),
            *entry.add(3),
        ]);
        // Read events at offset 4 (u16, little-endian).
        let events = u16::from_le_bytes([*entry.add(4), *entry.add(5)]);

        let revents: u16 = if fd < 0 {
            // Negative fd: skip; revents must be 0.
            0
        } else if guard.get(fd as usize).is_none() {
            POLLNVAL
        } else {
            // Simplified: report all requested events as immediately ready.
            // A full implementation would check pipe buffer fill, device state.
            events & (POLLIN | POLLOUT)
        };

        // Write revents at offset 6 (u16, little-endian).
        let rev_bytes = revents.to_le_bytes();
        *entry.add(6) = rev_bytes[0];
        *entry.add(7) = rev_bytes[1];

        if revents != 0 && revents != POLLNVAL {
            ready_count += 1;
        }
    }
    ready_count
}

struct FdSet {
    fds_bits: [u64; 16],
}

impl FdSet {
    /// Test whether `fd` is set in this fd_set.
    #[inline]
    fn is_set(&self, fd: usize) -> bool {
        if fd >= 1024 {
            return false;
        }
        (self.fds_bits[fd / 64] >> (fd % 64)) & 1 != 0
    }

    /// Set the bit for `fd`.
    #[inline]
    fn set(&mut self, fd: usize) {
        if fd < 1024 {
            self.fds_bits[fd / 64] |= 1u64 << (fd % 64);
        }
    }

    /// Zero all bits.
    #[inline]
    fn clear_all(&mut self) {
        self.fds_bits = [0u64; 16];
    }
}

/// POSIX struct timeval: seconds + microseconds since epoch or as a timeout.
///
/// Reference: POSIX.1-2017 §<sys/time.h>.
#[repr(C)]
struct TimeVal {
    tv_sec:  i64,
    tv_usec: i64,
}

/// `select(nfds, readfds_ptr, writefds_ptr, exceptfds_ptr, timeout_ptr) → nready | -errno`
///
/// Checks up to `nfds` file descriptors for readiness.
///   readfds:    fds to check for EPOLLIN  (data available to read).
///   writefds:   fds to check for EPOLLOUT (space available to write).
///   exceptfds:  fds to check for exceptional conditions (out-of-band data).
///               AF_UNIX sockets never carry OOB data, so this set is always
///               empty on return.  The pointer is validated if non-NULL.
///   timeout_ptr: *const TimeVal — NULL means block indefinitely; {0,0} means
///                poll once without blocking.
///
/// Returns the number of ready file descriptors, or a negative errno.
///
/// # Safety
/// Must be called with IRQs disabled (scheduler and VFS access).
pub(super) unsafe fn sys_select(
    nfds: i32,
    readfds_ptr: u64,
    writefds_ptr: u64,
    exceptfds_ptr: u64,
    timeout_ptr: u64,
) -> i64 {
    if nfds < 0 || nfds > 1024 {
        return EINVAL;
    }

    // --- Read input fd_sets from userspace ---

    let mut in_readfds = FdSet { fds_bits: [0u64; 16] };
    if readfds_ptr != 0 {
        if !validate_user_pointer(readfds_ptr, core::mem::size_of::<FdSet>()) {
            return EINVAL;
        }
        core::ptr::copy_nonoverlapping(
            readfds_ptr as *const u8,
            &mut in_readfds as *mut FdSet as *mut u8,
            core::mem::size_of::<FdSet>(),
        );
    }

    let mut in_writefds = FdSet { fds_bits: [0u64; 16] };
    if writefds_ptr != 0 {
        if !validate_user_pointer(writefds_ptr, core::mem::size_of::<FdSet>()) {
            return EINVAL;
        }
        core::ptr::copy_nonoverlapping(
            writefds_ptr as *const u8,
            &mut in_writefds as *mut FdSet as *mut u8,
            core::mem::size_of::<FdSet>(),
        );
    }

    // exceptfds: validate pointer.  No fd type supported by this kernel generates
    // OOB/exceptional data (AF_UNIX has no OOB), so the returned set will always
    // be empty.  We still validate the pointer to comply with POSIX error semantics.
    if exceptfds_ptr != 0 {
        if !validate_user_pointer(exceptfds_ptr, core::mem::size_of::<FdSet>()) {
            return EINVAL;
        }
    }

    // --- Read timeout ---

    let timeout_ticks: u64 = if timeout_ptr != 0 {
        if !validate_user_pointer(timeout_ptr, core::mem::size_of::<TimeVal>()) {
            return EINVAL;
        }
        let mut tv = TimeVal { tv_sec: 0, tv_usec: 0 };
        core::ptr::copy_nonoverlapping(
            timeout_ptr as *const u8,
            &mut tv as *mut TimeVal as *mut u8,
            core::mem::size_of::<TimeVal>(),
        );
        if tv.tv_sec == 0 && tv.tv_usec == 0 {
            // Non-blocking poll.
            0
        } else {
            let timeout_ms = (tv.tv_sec.max(0) as u64)
                .saturating_mul(1000)
                .saturating_add((tv.tv_usec.max(0) as u64) / 1000);
            let tick_interval_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
            (timeout_ms + tick_interval_ms - 1) / tick_interval_ms
        }
    } else {
        // NULL timeout → block indefinitely.
        u64::MAX
    };

    let is_nonblocking = timeout_ptr != 0 && timeout_ticks == 0;
    let deadline_tick = crate::platform::qemu_virt::timer::current_tick()
        .saturating_add(timeout_ticks);

    // --- Poll loop ---

    loop {
        let mut out_readfds  = FdSet { fds_bits: [0u64; 16] };
        let mut out_writefds = FdSet { fds_bits: [0u64; 16] };
        let mut ready_count: i64 = 0;

        for fd_index in 0..nfds as usize {
            if in_readfds.is_set(fd_index) {
                let readiness = crate::fs::epoll::check_fd_readiness_for_select(
                    fd_index as i32,
                    crate::fs::epoll::EPOLLIN,
                );
                if readiness & crate::fs::epoll::EPOLLIN != 0 {
                    out_readfds.set(fd_index);
                    ready_count += 1;
                }
            }
            if in_writefds.is_set(fd_index) {
                let readiness = crate::fs::epoll::check_fd_readiness_for_select(
                    fd_index as i32,
                    crate::fs::epoll::EPOLLOUT,
                );
                if readiness & crate::fs::epoll::EPOLLOUT != 0 {
                    out_writefds.set(fd_index);
                    ready_count += 1;
                }
            }
        }

        let timed_out = crate::platform::qemu_virt::timer::current_tick() >= deadline_tick;

        if ready_count > 0 || timed_out || is_nonblocking {
            // Write result fd_sets back to userspace.
            if readfds_ptr != 0 {
                core::ptr::copy_nonoverlapping(
                    &out_readfds as *const FdSet as *const u8,
                    readfds_ptr as *mut u8,
                    core::mem::size_of::<FdSet>(),
                );
            }
            if writefds_ptr != 0 {
                core::ptr::copy_nonoverlapping(
                    &out_writefds as *const FdSet as *const u8,
                    writefds_ptr as *mut u8,
                    core::mem::size_of::<FdSet>(),
                );
            }
            if exceptfds_ptr != 0 {
                // Exceptional conditions are not tracked; always write a zeroed set.
                let zeroed = FdSet { fds_bits: [0u64; 16] };
                core::ptr::copy_nonoverlapping(
                    &zeroed as *const FdSet as *const u8,
                    exceptfds_ptr as *mut u8,
                    core::mem::size_of::<FdSet>(),
                );
            }
            return ready_count;
        }

        // Not ready yet — yield and retry.
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.schedule();
        });
    }
}
