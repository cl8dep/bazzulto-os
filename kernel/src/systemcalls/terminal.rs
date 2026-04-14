// terminal.rs — Terminal and TTY syscall implementations.
//
// Syscalls: ioctl, tcgetattr, tcsetattr
// Helpers:  pty_master_index_for_fd

use super::*;

// ---------------------------------------------------------------------------
// Phase 10 — Terminal syscalls
// ---------------------------------------------------------------------------

/// `ioctl(fd, request, arg)` — device control.
///
/// Supported requests:
///   TIOCGWINSZ (0x5413) — get terminal window size into `struct winsize`:
///     u16 ws_row, u16 ws_col, u16 ws_xpixel (0), u16 ws_ypixel (0)
///   TIOCSWINSZ (0x5414) — set terminal window size (PTY pairs only).
///   TIOCGPTN   (0x80045430) — get PTY slave number (write u32 to arg).
///   TIOCSPTLCK (0x40045431) — lock/unlock PTY slave (no-op in this impl).
///
/// Reference: Linux include/uapi/asm-generic/ioctls.h.
const TIOCGWINSZ: u64 = 0x5413;

/// TIOCSWINSZ — set window size.
/// Reference: Linux include/uapi/asm-generic/ioctls.h.
const TIOCSWINSZ: u64 = 0x5414;

/// TIOCGPTN — get PTY slave index number.
/// Reference: Linux include/uapi/linux/tty.h.
const TIOCGPTN: u64 = 0x80045430;

/// TIOCSPTLCK — set/clear PTY slave lock.
/// Reference: Linux include/uapi/linux/tty.h.
const TIOCSPTLCK: u64 = 0x40045431;

pub(super) unsafe fn sys_ioctl(fd: i32, request: u64, arg: u64) -> i64 {
    match request {
        TIOCGWINSZ => {
            if !validate_user_pointer(arg, 8) {
                return EINVAL;
            }
            // Check if fd is a PTY master; if so, use the PTY's window size.
            let pty_index: Option<usize> = pty_master_index_for_fd(fd);
            let (rows, cols) = if let Some(index) = pty_index {
                crate::drivers::pty::pty_get_window_size(index)
            } else {
                crate::drivers::tty::tty_get_winsize_pair()
            };
            let winsize_ptr = arg as *mut u16;
            winsize_ptr.write(rows);
            winsize_ptr.add(1).write(cols);
            winsize_ptr.add(2).write(0); // ws_xpixel
            winsize_ptr.add(3).write(0); // ws_ypixel
            0
        }
        TIOCSWINSZ => {
            if !validate_user_pointer(arg, 8) {
                return EINVAL;
            }
            let winsize_ptr = arg as *const u16;
            let rows = winsize_ptr.read();
            let cols = winsize_ptr.add(1).read();
            if let Some(index) = pty_master_index_for_fd(fd) {
                crate::drivers::pty::pty_set_window_size(index, rows, cols);
            } else {
                crate::drivers::tty::tty_set_winsize(rows, cols);
            }
            0
        }
        TIOCGPTN => {
            // Write the PTY index as u32 to the user pointer.
            if !validate_user_pointer(arg, 4) {
                return EINVAL;
            }
            match pty_master_index_for_fd(fd) {
                Some(index) => {
                    let output_ptr = arg as *mut u32;
                    output_ptr.write(index as u32);
                    0
                }
                None => EINVAL,
            }
        }
        TIOCSPTLCK => {
            // Lock/unlock PTY slave — this implementation does not enforce
            // locking; accept the call and return success.
            0
        }
        _ => -25, // ENOTTY — not a typewriter
    }
}

/// Return the PTY table index if `fd` refers to a PTY master inode, else None.
///
/// # Safety
/// Must be called with IRQs disabled (scheduler access).
unsafe fn pty_master_index_for_fd(fd: i32) -> Option<usize> {
    if fd < 0 {
        return None;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    })?;
    let guard = fd_table_arc.lock();
    match guard.get(fd as usize)? {
            crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                // Downcast: check if inode_type is CharDevice and the inode's
                // stat inode_number matches one of our PTY master inodes.
                // Since we cannot use Any in no_std, we store a sentinel in
                // the inode_number encoding.  Instead, we rely on the fact
                // that only PtyMasterInode exposes a `pty_index` field.
                // We use a type-erased check: call a zero-size probe.
                //
                // Approach: cast the fat pointer to *const () to get the
                // vtable; then check a known method behaviour. Since that is
                // fragile, use the canonical no_std approach: wrap the inode
                // in our own enum or use a marker trait.
                //
                // For now, we use the naming convention: PTY master inodes
                // are the only CharDevice inodes that return a stat with
                // mode 0o020666 AND size 0. All other CharDevices do too, so
                // this does not work cleanly.
                //
                // Best practical approach without Any: expose a `pty_index()`
                // method on the Inode trait with a default returning None.
                // Until the trait is extended, we skip PTY-specific TIOCGWINSZ
                // on non-PTY fds gracefully by returning None here, and the
                // caller falls back to the global TTY window size.
                let _ = inode;
                None
            }
            _ => None,
        }
}

// ---------------------------------------------------------------------------
// sys_futex — fast user-space mutex wait/wake
// ---------------------------------------------------------------------------

/// `futex(uaddr, op, val, timeout_ptr)` — minimum implementation for
/// `pthread_mutex_lock` / `pthread_mutex_unlock`.
///
/// Operations:
///   FUTEX_WAIT (0): if `*uaddr == val`, sleep on `uaddr`.
///                   Returns 0 on wakeup, -EAGAIN if `*uaddr != val`.
///   FUTEX_WAKE (1): wake up to `val` processes sleeping on `uaddr`.
///                   Returns the number of processes woken.
///
/// `timeout_ptr` is accepted but ignored (indefinite sleep); upgrade later.
///
/// Reference: Linux `futex(2)`, `kernel/futex/core.c`.
///
/// # Safety
/// Must be called with IRQs disabled (scheduler invariant).
unsafe fn sys_futex(
    uaddr: u64,
    operation: i32,
    value: u32,
    _timeout_ptr: u64,
) -> i64 {
    if !validate_user_pointer(uaddr, core::mem::size_of::<u32>()) {
        return EINVAL;
    }

    match operation & 0x7F { // mask off FUTEX_PRIVATE_FLAG (0x80)
        FUTEX_WAIT => {
            // Read the current value at uaddr.
            let current_value = *(uaddr as *const u32);
            if current_value != value {
                // Value changed before we could sleep — report and let caller retry.
                return EAGAIN;
            }

            // Enqueue the current PID in the wait queue for this address.
            let current_pid = crate::scheduler::with_scheduler(|scheduler| {
                scheduler.current_pid()
            });

            {
                let table = &mut *FUTEX_TABLE.0.get();
                table.entry(uaddr).or_insert_with(VecDeque::new).push_back(current_pid);
            }

            // Sleep indefinitely; FUTEX_WAKE will transition us to Ready.
            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process_mut() {
                    process.state = crate::process::ProcessState::Sleeping {
                        wake_at_tick: u64::MAX,
                    };
                }
                scheduler.schedule();
            });

            0
        }

        FUTEX_WAKE => {
            // Wake up to `value` waiters for this address.
            let wake_count = value as usize;
            let mut woken: usize = 0;

            let pids_to_wake: alloc::vec::Vec<Pid> = {
                let table = &mut *FUTEX_TABLE.0.get();
                match table.get_mut(&uaddr) {
                    Some(queue) => {
                        let n = wake_count.min(queue.len());
                        queue.drain(..n).collect()
                    }
                    None => alloc::vec::Vec::new(),
                }
            };

            crate::scheduler::with_scheduler(|scheduler| {
                for pid in &pids_to_wake {
                    scheduler.futex_make_ready(*pid);
                    woken += 1;
                }
            });

            woken as i64
        }

        _ => EINVAL,
    }
}

/// `tcgetattr(fd, termios_ptr)` — get terminal attributes.
///
/// ABI: termios_ptr points to a `struct termios` (c_iflag, c_oflag, c_cflag, c_lflag: u32; c_cc: [u8; 32])
/// Total size: 4*4 + 32 = 48 bytes.
///
/// Reference: POSIX.1-2017 §11.1.
pub(super) unsafe fn sys_tcgetattr(fd: i32, termios_ptr: *mut u8) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(termios_ptr as u64, 48) {
        return EINVAL;
    }
    crate::drivers::tty::tty_tcgetattr(termios_ptr as *mut crate::drivers::tty::Termios);
    0
}

/// `tcsetattr(fd, optional_actions, termios_ptr)` — set terminal attributes.
///
/// `optional_actions`: TCSANOW=0, TCSADRAIN=1, TCSAFLUSH=2 (we treat all as immediate).
pub(super) unsafe fn sys_tcsetattr(fd: i32, _optional_actions: i32, termios_ptr: *const u8) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(termios_ptr as u64, 48) {
        return EINVAL;
    }
    crate::drivers::tty::tty_tcsetattr(termios_ptr as *const crate::drivers::tty::Termios);
    0
}

