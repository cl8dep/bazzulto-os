// systemcalls/mod.rs — System call dispatch and shared infrastructure.
//
// ┌─────────────────────────────────────────────────────────────────────┐
// │  SYSCALL ABI — FROZEN AT v0.2                                      │
// │                                                                     │
// │  Syscall numbers 0–161 are immutable.                               │
// │  To add a new syscall, use the next available number >= 162.        │
// │  Never reassign an existing number.                                 │
// │                                                                     │
// │  Authoritative reference: docs/wiki/System-Calls.md                │
// └─────────────────────────────────────────────────────────────────────┘
//
// Syscall ABI (AAPCS64 + Linux-compatible convention):
//   x8  = syscall number
//   x0–x5 = arguments (arg0..arg5)
//   x0  = return value on exit
//
// All handlers take (*mut ExceptionFrame) and modify frame.x[0] to set the
// return value.
//
// Reference:
//   Linux kernel arch/arm64/kernel/syscall.c — syscall entry.
//   POSIX.1-2017 — syscall semantics.

use crate::arch::arm64::exceptions::ExceptionFrame;

// ---------------------------------------------------------------------------
// Syscall number constants
// ---------------------------------------------------------------------------

pub mod numbers {
    pub const EXIT:          u64 = 0;
    pub const WRITE:         u64 = 1;
    pub const READ:          u64 = 2;
    pub const YIELD:         u64 = 3;
    pub const OPEN:          u64 = 4;
    pub const CLOSE:         u64 = 5;
    pub const SEEK:          u64 = 6;
    pub const SPAWN:         u64 = 7;
    pub const LIST:          u64 = 8;
    pub const WAIT:          u64 = 9;
    pub const PIPE:          u64 = 10;
    pub const DUP:           u64 = 11;
    pub const DUP2:          u64 = 12;
    pub const MMAP:          u64 = 13;
    pub const MUNMAP:        u64 = 14;
    pub const FORK:          u64 = 15;
    pub const EXEC:          u64 = 16;
    pub const GETPID:        u64 = 17;
    pub const GETPPID:       u64 = 18;
    pub const CLOCK_GETTIME: u64 = 19;
    pub const NANOSLEEP:     u64 = 20;
    pub const SIGACTION:     u64 = 21;
    pub const KILL:          u64 = 22;
    pub const SIGRETURN:     u64 = 23;
    pub const CREAT:         u64 = 24;
    pub const UNLINK:        u64 = 25;
    pub const FSTAT:         u64 = 26;
    pub const SETFGPID:      u64 = 27;
    pub const DISK_INFO:     u64 = 28;
    pub const GETRANDOM:     u64 = 29;
    pub const NICE:          u64 = 30;
    pub const GETPRIORITY:   u64 = 31;
    pub const SETPRIORITY:   u64 = 32;
    pub const GETRLIMIT:     u64 = 33;
    pub const SETRLIMIT:     u64 = 34;
    pub const GETPGRP:       u64 = 35;
    pub const SETPGID:       u64 = 36;
    pub const GETSID:        u64 = 37;
    pub const SETSID:        u64 = 38;
    pub const TCGETPGRP:     u64 = 39;
    pub const TCSETPGRP:     u64 = 40;
    // Phase 8 — POSIX syscalls
    pub const UNAME:         u64 = 41;
    pub const SYSINFO:       u64 = 42;
    pub const SIGPROCMASK:   u64 = 43;
    pub const SIGPENDING:    u64 = 44;
    pub const SIGSUSPEND:    u64 = 45;
    pub const GETRUSAGE:     u64 = 46;
    pub const PRCTL:         u64 = 47;
    pub const GETTIMEOFDAY:  u64 = 48;
    pub const POLL:          u64 = 49;
    pub const GETUID:        u64 = 50;
    pub const GETGID:        u64 = 51;
    // Phase 9 — VFS syscalls
    pub const CHDIR:         u64 = 52;
    pub const GETCWD:        u64 = 53;
    pub const MKDIR:         u64 = 54;
    pub const RMDIR:         u64 = 55;
    pub const RENAME:        u64 = 56;
    pub const GETDENTS64:    u64 = 57;
    pub const TRUNCATE:      u64 = 58;
    pub const FSYNC:         u64 = 59;
    // Phase 10 — Terminal / TTY
    pub const IOCTL:         u64 = 60;
    pub const TCGETATTR:     u64 = 61;
    pub const TCSETATTR:     u64 = 62;
    // Phase 11 — Futex / threading primitives
    pub const FUTEX:         u64 = 63;
    // Phase 12 — epoll
    pub const EPOLL_CREATE1: u64 = 64;
    pub const EPOLL_CTL:     u64 = 65;
    pub const EPOLL_WAIT:    u64 = 66;
    // Phase 13 — POSIX threads primitives
    pub const CLONE:         u64 = 67;
    pub const SET_TLS:       u64 = 68;
    pub const GETTID:        u64 = 69;
    // Display
    pub const FRAMEBUFFER_MAP: u64 = 70;
    // Phase 14 — FIFOs (named pipes) and POSIX semaphores
    pub const MKFIFO:       u64 = 71;
    pub const SEM_OPEN:     u64 = 72;
    pub const SEM_CLOSE:    u64 = 73;
    pub const SEM_WAIT:     u64 = 74;
    pub const SEM_TRYWAIT:  u64 = 75;
    pub const SEM_POST:     u64 = 76;
    pub const SEM_UNLINK:   u64 = 77;
    pub const SEM_GETVALUE: u64 = 78;
    // Phase 15 — Unix domain sockets
    pub const SOCKET:      u64 = 79;
    pub const BIND:        u64 = 80;
    pub const LISTEN:      u64 = 81;
    pub const ACCEPT:      u64 = 82;
    pub const CONNECT:     u64 = 83;
    pub const SEND:        u64 = 84;
    pub const RECV:        u64 = 85;
    pub const SHUTDOWN:    u64 = 86;
    pub const GETSOCKNAME: u64 = 87;
    pub const GETPEERNAME: u64 = 88;
    pub const SOCKETPAIR:  u64 = 89;
    // Phase 15 — POSIX message queues
    pub const MQ_OPEN:    u64 = 90;
    pub const MQ_CLOSE:   u64 = 91;
    pub const MQ_SEND:    u64 = 92;
    pub const MQ_RECEIVE: u64 = 93;
    pub const MQ_UNLINK:  u64 = 94;
    pub const MQ_GETATTR: u64 = 95;
    // Phase 16 — POSIX I/O multiplexing
    pub const SELECT:     u64 = 96;
    // Phase 17 — Process model extensions
    pub const UMASK:            u64 = 97;
    pub const SIGALTSTACK:      u64 = 98;
    // Phase 18 — POSIX alarm + machine control
    pub const ALARM:            u64 = 99;
    pub const MACHINE_REBOOT:   u64 = 100;
    pub const MACHINE_POWEROFF: u64 = 101;
    // Phase 19 — POSIX UID/GID
    pub const GETEUID:  u64 = 102;
    pub const GETEGID:  u64 = 103;
    pub const SETUID:   u64 = 104;
    pub const SETGID:   u64 = 105;
    pub const SETEUID:  u64 = 106;
    pub const SETEGID:  u64 = 107;
    pub const CHMOD:    u64 = 108;
    pub const FCHMOD:   u64 = 109;
    pub const CHOWN:    u64 = 110;
    pub const FCHOWN:   u64 = 111;
    pub const LCHOWN:   u64 = 112;
    pub const MOUNT:      u64 = 113;
    pub const GETMOUNTS:  u64 = 114;
    // musl/Linux ABI compatibility (numbers 115–161)
    pub const SET_TID_ADDRESS:  u64 = 115;
    pub const SET_ROBUST_LIST:  u64 = 116;
    pub const GET_ROBUST_LIST:  u64 = 117;
    pub const EXIT_GROUP:       u64 = 118;
    pub const BRK:              u64 = 119;
    pub const OPENAT:           u64 = 120;
    pub const FSTATAT:          u64 = 121;
    pub const UNLINKAT:         u64 = 122;
    pub const MKDIRAT:          u64 = 123;
    pub const FTRUNCATE:        u64 = 124;
    pub const FDATASYNC:        u64 = 125;
    pub const PIPE2:            u64 = 126;
    pub const DUP3:             u64 = 127;
    pub const FCNTL:            u64 = 128;
    pub const MPROTECT:         u64 = 129;
    pub const ACCESS:           u64 = 130;
    pub const READLINK:         u64 = 131;
    pub const CLOCK_NANOSLEEP:  u64 = 132;
    pub const CLOCK_GETRES:     u64 = 133;
    pub const WAITID:           u64 = 134;
    pub const TKILL:            u64 = 135;
    pub const TGKILL:           u64 = 136;
    pub const MREMAP:           u64 = 137;
    pub const MADVISE:          u64 = 138;
    pub const MSYNC:            u64 = 139;
    pub const SYMLINK:          u64 = 140;
    pub const LINK:             u64 = 141;
    pub const READLINKAT:       u64 = 142;
    pub const FCHOWNAT:         u64 = 143;
    pub const FCHMODAT:         u64 = 144;
    pub const FCHDIR:           u64 = 145;
    pub const STATX:            u64 = 146;
    pub const READV:            u64 = 147;
    pub const WRITEV:           u64 = 148;
    pub const PREAD64:          u64 = 149;
    pub const PWRITE64:         u64 = 150;
    pub const RENAMEAT:         u64 = 151;
    pub const TIMES:            u64 = 152;
    pub const GETGROUPS:        u64 = 153;
    pub const GETPGID:          u64 = 154;
    pub const CLOCK_SETTIME:    u64 = 155;
    pub const TIMER_CREATE:     u64 = 156;
    pub const TIMER_SETTIME:    u64 = 157;
    pub const TIMER_GETTIME:    u64 = 158;
    pub const TIMER_DELETE:     u64 = 159;
    pub const SETITIMER:        u64 = 160;
    pub const GETITIMER:        u64 = 161;
    // SCM_RIGHTS support — added in v0.3 (M2)
    pub const SENDMSG:          u64 = 162;
    pub const RECVMSG:          u64 = 163;
}

use numbers::*;

// ---------------------------------------------------------------------------
// Submodule declarations
// ---------------------------------------------------------------------------

mod process;
mod io;
mod memory;
mod signals;
mod time;
mod scheduler;
mod vfs;
mod terminal;
mod threads;
mod identity;
mod system;
mod multiplexing;
pub mod posix_abi;

use self::process::*;
use self::io::*;
use self::memory::*;
use self::signals::*;
use self::time::*;
use self::scheduler::*;
// Re-export terminal_foreground_pgid for external callers (e.g., drivers/tty.rs).
pub use self::scheduler::terminal_foreground_pgid;
use self::vfs::*;
use self::terminal::*;
use self::threads::*;
use self::identity::*;
use self::system::*;
use self::multiplexing::*;

// ---------------------------------------------------------------------------
// Error codes (negated POSIX errno values)
// ---------------------------------------------------------------------------

const EPERM:    i64 = -1;
const ENOENT:   i64 = -2;
const ESRCH:    i64 = -3;
const EINTR:    i64 = -4;
const ENOEXEC:  i64 = -8;
const EBADF:    i64 = -9;
const ECHILD:   i64 = -10;
const EAGAIN:   i64 = -11;
const ENOMEM:   i64 = -12;
const EACCES:   i64 = -13;
const EFAULT:   i64 = -14;
const EEXIST:   i64 = -17;
const ENOTDIR:  i64 = -20;
const EINVAL:   i64 = -22;
const EMFILE:   i64 = -24;
const EPIPE:    i64 = -32;
const ENOSYS:   i64 = -38;
const ESPIPE:   i64 = -29;

// ---------------------------------------------------------------------------
// mmap flags (Linux ABI values for AArch64)
// ---------------------------------------------------------------------------

/// MAP_SHARED — changes are visible to other processes mapping the same region.
/// Linux value: 0x01.
const MAP_SHARED: i32 = 0x01;

/// MAP_ANONYMOUS — not backed by a file; contents initialised to zero.
/// Linux value: 0x20.
const MAP_ANONYMOUS: i32 = 0x20;

// ---------------------------------------------------------------------------
// open(2) flags (Linux ABI values for AArch64)
// ---------------------------------------------------------------------------

/// O_CREAT — create the file if it does not exist.
/// Linux AArch64 value: 0x40.
const O_CREAT: i32 = 0x40;

/// O_EXCL — fail with EEXIST if the file already exists (used with O_CREAT).
/// Linux AArch64 value: 0x80.
const O_EXCL: i32 = 0x80;

/// O_TRUNC — truncate the file to zero length on open.
/// Linux AArch64 value: 0x200.
const O_TRUNC: i32 = 0x200;

/// O_NONBLOCK — non-blocking I/O.  On pipes, returns EAGAIN instead of blocking.
/// Linux AArch64 value: 0x800.
const O_NONBLOCK: i32 = 0x800;

/// O_CLOEXEC — close FD on exec().
/// Linux AArch64 value: 0x80000.
const O_CLOEXEC: i32 = 0x80000;

// ---------------------------------------------------------------------------
// futex operation codes (Linux ABI values)
// ---------------------------------------------------------------------------

/// FUTEX_WAIT — sleep if `*uaddr == val`.
const FUTEX_WAIT: i32 = 0;

/// FUTEX_WAKE — wake up to `val` waiters on `uaddr`.
const FUTEX_WAKE: i32 = 1;

// ---------------------------------------------------------------------------
// AT_* constants for *at() syscalls (Linux ABI values)
// ---------------------------------------------------------------------------

/// AT_FDCWD — special dirfd value meaning "use process current working directory".
/// Linux value: -100 (as i32).
const AT_FDCWD: i32 = -100;

/// AT_REMOVEDIR — flag for unlinkat: act like rmdir rather than unlink.
/// Linux value: 0x200.
const AT_REMOVEDIR: i32 = 0x200;

// ---------------------------------------------------------------------------
// FutexTable — global wait queue for futex(2)
// ---------------------------------------------------------------------------

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use core::cell::UnsafeCell;

use crate::process::Pid;

/// Per-address queue of sleeping process IDs.
///
/// Key: user virtual address (u64, page-aligned not required — exact address
///   as passed by the futex syscall).
/// Value: ordered queue of PIDs waiting on that address (FIFO wake order).
struct FutexTable(UnsafeCell<BTreeMap<u64, VecDeque<Pid>>>);

// SAFETY: Bazzulto OS is single-core with IRQs disabled during all kernel
// operations. There is never concurrent access to FutexTable from multiple
// hardware threads.
unsafe impl Sync for FutexTable {}

static FUTEX_TABLE: FutexTable = FutexTable(UnsafeCell::new(BTreeMap::new()));

// ---------------------------------------------------------------------------
// SharedRegionTable — tracking for MAP_SHARED | MAP_ANONYMOUS regions
// ---------------------------------------------------------------------------

/// Metadata for a MAP_SHARED | MAP_ANONYMOUS mapping.
///
/// On fork(), regions present in this table are mapped directly into both
/// parent and child with the same physical pages (no CoW), so writes by one
/// process are immediately visible to the other.
///
/// `phys_base` is a placeholder; actual physical frames are tracked by the
/// process page table.  The table is used here only to identify which virtual
/// address ranges are shared so the fork path can skip CoW marking.
pub struct SharedRegion {
    /// Physical base address of the first page (informational; may be 0).
    pub phys_base: u64,
    /// Number of 4 KiB pages in the region.
    pub page_count: usize,
    /// Number of processes currently mapping this region.
    pub reference_count: u32,
}

struct SharedRegionTable(UnsafeCell<BTreeMap<u64, SharedRegion>>);

// SAFETY: Single-core, IRQs disabled during kernel operations.
unsafe impl Sync for SharedRegionTable {}

static SHARED_REGION_TABLE: SharedRegionTable =
    SharedRegionTable(UnsafeCell::new(BTreeMap::new()));

/// Return true if `virtual_address` is the base of a MAP_SHARED | MAP_ANONYMOUS region.
pub fn is_shared_anonymous_region(virtual_address: u64) -> bool {
    let table = unsafe { &*SHARED_REGION_TABLE.0.get() };
    table.contains_key(&virtual_address)
}

// ---------------------------------------------------------------------------
// User pointer validation
// ---------------------------------------------------------------------------

/// Return `true` if the range `[ptr, ptr + len)` is entirely within
/// user virtual address space and does not start at the null page.
///
/// Checks:
///   1. `ptr` is not null (>= PAGE_SIZE = 4096).
///   2. `ptr + len` does not overflow u64.
///   3. `ptr + len <= USER_ADDR_LIMIT` (2^48 on AArch64 with T0SZ=16).
///
/// This does NOT verify that the pages are mapped — that is enforced by the
/// hardware MMU. Its purpose is to reject clearly invalid pointers before
/// dereferencing them in the kernel.
///
/// Reference: Linux arch/arm64/include/asm/uaccess.h `access_ok()`.
#[inline]
pub(crate) fn validate_user_pointer(ptr: u64, len: usize) -> bool {
    const PAGE_SIZE: u64 = 4096;
    let end = ptr.wrapping_add(len as u64);
    ptr >= PAGE_SIZE
        && end >= ptr                              // no overflow
        && end <= crate::process::USER_ADDR_LIMIT
}

/// Copy `kernel_dst.len()` bytes from user address `user_src` into `kernel_dst`.
///
/// Returns `Ok(())` on success or `Err(EFAULT)` if the pointer range is invalid.
///
/// This is the canonical way to read user memory from syscall handlers.
/// All direct `core::slice::from_raw_parts(user_ptr, len)` patterns should be
/// replaced with this function for consistency and security.
///
/// Reference: Linux `copy_from_user()` (arch/arm64/include/asm/uaccess.h).
#[inline]
pub(crate) unsafe fn copy_from_user(user_src: *const u8, kernel_dst: &mut [u8]) -> Result<(), i64> {
    let len = kernel_dst.len();
    if len == 0 {
        return Ok(());
    }
    if !validate_user_pointer(user_src as u64, len) {
        return Err(EFAULT);
    }
    let src_slice = core::slice::from_raw_parts(user_src, len);
    kernel_dst.copy_from_slice(src_slice);
    Ok(())
}

/// Copy `kernel_src` bytes to user address `user_dst`.
///
/// Returns `Ok(())` on success or `Err(EFAULT)` if the pointer range is invalid.
///
/// This is the canonical way to write to user memory from syscall handlers.
/// All direct `core::slice::from_raw_parts_mut(user_ptr, len)` patterns should
/// be replaced with this function for consistency and security.
///
/// Reference: Linux `copy_to_user()` (arch/arm64/include/asm/uaccess.h).
#[inline]
pub(crate) unsafe fn copy_to_user(user_dst: *mut u8, kernel_src: &[u8]) -> Result<(), i64> {
    let len = kernel_src.len();
    if len == 0 {
        return Ok(());
    }
    if !validate_user_pointer(user_dst as u64, len) {
        return Err(EFAULT);
    }
    let dst_slice = core::slice::from_raw_parts_mut(user_dst, len);
    dst_slice.copy_from_slice(kernel_src);
    Ok(())
}

/// Read a single value of type T from user address space.
///
/// Returns `Ok(value)` or `Err(EFAULT)` if the pointer is invalid.
#[inline]
pub(crate) unsafe fn get_user<T: Copy>(user_ptr: *const T) -> Result<T, i64> {
    if !validate_user_pointer(user_ptr as u64, core::mem::size_of::<T>()) {
        return Err(EFAULT);
    }
    Ok(core::ptr::read_volatile(user_ptr))
}

/// Write a single value of type T to user address space.
///
/// Returns `Ok(())` or `Err(EFAULT)` if the pointer is invalid.
#[inline]
pub(crate) unsafe fn put_user<T: Copy>(user_ptr: *mut T, value: T) -> Result<(), i64> {
    if !validate_user_pointer(user_ptr as u64, core::mem::size_of::<T>()) {
        return Err(EFAULT);
    }
    core::ptr::write_volatile(user_ptr, value);
    Ok(())
}

/// Copy a NUL-terminated string from user address space into `buf`.
///
/// Returns `Some(len)` where `len` is the number of bytes written to `buf`
/// (not counting the NUL terminator), or `None` if the pointer is invalid
/// or the string exceeds 511 bytes.
pub(crate) unsafe fn copy_user_cstr(ptr: *const u8, buf: &mut [u8; 512]) -> Option<usize> {
    const PAGE_SIZE: u64 = 4096;
    if (ptr as u64) < PAGE_SIZE || (ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return None;
    }
    let mut i = 0usize;
    loop {
        if i >= 511 {
            return None;
        }
        let byte = core::ptr::read_volatile(ptr.add(i));
        if byte == 0 {
            buf[i] = 0;
            return Some(i);
        }
        buf[i] = byte;
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// dispatch — called from exception_handler_sync_el0 on SVC
// ---------------------------------------------------------------------------

/// Dispatch a syscall from the exception frame.
///
/// # Safety
/// `frame` must point to the ExceptionFrame on the current process's kernel
/// stack.  The frame must be valid for the lifetime of this function.
/// Called with IRQs still masked at EL1 (DAIF.I = 1 from the vector entry).
pub fn dispatch(frame: *mut ExceptionFrame, syscall_number: u64) {
    let arg0 = unsafe { (*frame).x[0] };
    let arg1 = unsafe { (*frame).x[1] };
    let arg2 = unsafe { (*frame).x[2] };
    let arg3 = unsafe { (*frame).x[3] };
    let arg4 = unsafe { (*frame).x[4] };
    let arg5 = unsafe { (*frame).x[5] };

    // Increment per-process kernel time counter.  Each syscall counts as one
    // tick regardless of actual duration.  Used by getrusage() ru_stime.
    unsafe {
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process_mut() {
                process.sys_time_ticks = process.sys_time_ticks.saturating_add(1);
            }
        });
    }

    let return_value: i64 = unsafe {
        match syscall_number {
            EXIT          => sys_exit(arg0 as i32),
            WRITE         => sys_write(arg0 as i32, arg1 as *const u8, arg2 as usize),
            READ          => sys_read(arg0 as i32, arg1 as *mut u8, arg2 as usize),
            YIELD         => sys_yield(),
            OPEN          => sys_open(arg0 as *const u8, arg1 as usize, arg2 as i32, arg3 as u32),
            CLOSE         => sys_close(arg0 as i32),
            SEEK          => sys_seek(arg0 as i32, arg1 as i64, arg2 as i32),
            SPAWN         => sys_spawn(arg0 as *const u8, arg1),
            LIST          => sys_list(arg0 as *mut u8, arg1 as usize),
            WAIT          => sys_wait(arg0 as i32, arg1 as *mut i32, arg2 as i32, arg3),
            PIPE          => sys_pipe(arg0 as *mut i32),
            DUP           => sys_dup(arg0 as i32),
            DUP2          => sys_dup2(arg0 as i32, arg1 as i32),
            MMAP          => sys_mmap(arg0, arg1, arg2 as i32, arg3 as i32, arg4 as i32, arg5),
            MUNMAP        => sys_munmap(arg0, arg1),
            FORK          => sys_fork(frame),
            EXEC          => sys_exec(frame, arg0 as *const u8, arg1, arg2),
            GETPID        => sys_getpid(),
            GETPPID       => sys_getppid(),
            CLOCK_GETTIME => sys_clock_gettime(arg0 as i32, arg1 as *mut u64),
            NANOSLEEP     => sys_nanosleep(arg0 as *const u64, arg1 as *mut u64),
            SIGACTION     => sys_sigaction(arg0 as i32, arg1, arg2, arg3 as usize),
            KILL          => sys_kill(arg0 as i32, arg1 as i32),
            SIGRETURN     => sys_sigreturn(frame),
            CREAT         => sys_creat(arg0 as *const u8, arg1 as u32),
            UNLINK        => sys_unlink(arg0 as *const u8),
            FSTAT         => sys_fstat(arg0 as i32, arg1 as *mut u8),
            SETFGPID      => sys_setfgpid(arg0 as i32),
            DISK_INFO     => sys_disk_info(arg0 as *mut u64),
            GETRANDOM     => sys_getrandom(arg0 as *mut u8, arg1 as usize, arg2 as u32),
            NICE          => sys_nice(arg0 as i32),
            GETPRIORITY   => sys_getpriority(arg0 as i32, arg1 as i32),
            SETPRIORITY   => sys_setpriority(arg0 as i32, arg1 as i32, arg2 as i32),
            GETRLIMIT     => sys_getrlimit(arg0 as u32, arg1 as *mut u64),
            SETRLIMIT     => sys_setrlimit(arg0 as u32, arg1 as *const u64),
            GETPGRP       => sys_getpgrp(),
            SETPGID       => sys_setpgid(arg0 as i32, arg1 as i32),
            GETSID        => sys_getsid(arg0 as i32),
            SETSID        => sys_setsid(),
            TCGETPGRP     => sys_tcgetpgrp(arg0 as i32),
            TCSETPGRP     => sys_tcsetpgrp(arg0 as i32, arg1 as i32),
            // Phase 8
            UNAME         => sys_uname(arg0 as *mut u8),
            SYSINFO       => sys_sysinfo(arg0 as *mut u64),
            SIGPROCMASK   => sys_sigprocmask(arg0 as i32, arg1 as *const u64, arg2 as *mut u64, arg3 as usize),
            SIGPENDING    => sys_sigpending(arg0 as *mut u64, arg1 as usize),
            SIGSUSPEND    => sys_sigsuspend(frame, arg0 as *const u64, arg1 as usize),
            GETRUSAGE     => sys_getrusage(arg0 as i32, arg1 as *mut u64),
            PRCTL         => sys_prctl(arg0 as i32, arg1 as *const u8, arg2 as usize),
            GETTIMEOFDAY  => sys_gettimeofday(arg0 as *mut u64, arg1),
            POLL          => sys_poll(arg0 as *mut u8, arg1 as usize, arg2 as i32),
            GETUID        => sys_getuid(),
            GETGID        => sys_getgid(),
            // Phase 10 — Terminal
            IOCTL         => sys_ioctl(arg0 as i32, arg1, arg2),
            TCGETATTR     => sys_tcgetattr(arg0 as i32, arg1 as *mut u8),
            TCSETATTR     => sys_tcsetattr(arg0 as i32, arg1 as i32, arg2 as *const u8),
            // Phase 11 — Futex / threading
            FUTEX         => sys_futex(arg0, arg1 as i32, arg2 as u32, arg3),
            // Phase 12 — epoll
            EPOLL_CREATE1 => sys_epoll_create1(arg0 as i32),
            EPOLL_CTL     => sys_epoll_ctl(arg0 as i32, arg1 as i32, arg2 as i32, arg3),
            EPOLL_WAIT    => sys_epoll_wait(arg0 as i32, arg1, arg2 as i32, arg3 as i32),
            // Phase 13 — POSIX threads primitives
            CLONE         => sys_clone(frame, arg0, arg1, arg2, arg3, arg4),
            SET_TLS       => sys_set_tls(arg0),
            GETTID        => sys_gettid(),
            // Display
            FRAMEBUFFER_MAP => sys_framebuffer_map(arg0 as *mut u64),
            // Phase 9 — VFS syscalls
            CHDIR         => sys_chdir(arg0 as *const u8),
            GETCWD        => sys_getcwd(arg0 as *mut u8, arg1 as usize),
            MKDIR         => sys_mkdir(arg0 as *const u8, arg1 as u32),
            RMDIR         => sys_rmdir(arg0 as *const u8),
            RENAME        => sys_rename(arg0 as *const u8, arg1 as *const u8),
            GETDENTS64    => sys_getdents64(arg0 as i32, arg1 as *mut u8, arg2 as usize),
            TRUNCATE      => sys_truncate(arg0 as *const u8, arg1),
            FSYNC         => sys_fsync(arg0 as i32),
            // Phase 14 — FIFOs and POSIX semaphores
            MKFIFO        => sys_mkfifo(arg0 as *const u8, arg1 as u32),
            SEM_OPEN      => crate::ipc::sem::sys_sem_open(arg0, arg1 as usize, arg2 as i32, arg3 as u32),
            SEM_CLOSE     => crate::ipc::sem::sys_sem_close(arg0 as i32),
            SEM_WAIT      => crate::ipc::sem::sys_sem_wait(arg0 as i32),
            SEM_TRYWAIT   => crate::ipc::sem::sys_sem_trywait(arg0 as i32),
            SEM_POST      => crate::ipc::sem::sys_sem_post(arg0 as i32),
            SEM_UNLINK    => crate::ipc::sem::sys_sem_unlink(arg0, arg1 as usize),
            SEM_GETVALUE  => crate::ipc::sem::sys_sem_getvalue(arg0 as i32, arg1),
            // Phase 15 — Unix domain sockets
            SOCKET      => crate::ipc::socket::sys_socket(arg0 as i32, arg1 as i32, arg2 as i32),
            BIND        => crate::ipc::socket::sys_bind(arg0 as i32, arg1, arg2 as usize),
            LISTEN      => crate::ipc::socket::sys_listen(arg0 as i32, arg1 as i32),
            ACCEPT      => crate::ipc::socket::sys_accept(arg0 as i32, arg1, arg2),
            CONNECT     => crate::ipc::socket::sys_connect(arg0 as i32, arg1, arg2 as usize),
            SEND        => crate::ipc::socket::sys_send(arg0 as i32, arg1, arg2 as usize, arg3 as i32),
            RECV        => crate::ipc::socket::sys_recv(arg0 as i32, arg1, arg2 as usize, arg3 as i32),
            SHUTDOWN    => crate::ipc::socket::sys_shutdown(arg0 as i32, arg1 as i32),
            GETSOCKNAME => crate::ipc::socket::sys_getsockname(arg0 as i32, arg1, arg2),
            GETPEERNAME => crate::ipc::socket::sys_getpeername(arg0 as i32, arg1, arg2),
            SOCKETPAIR  => crate::ipc::socket::sys_socketpair(arg0 as i32, arg1 as i32, arg2 as i32, arg3),
            // Phase 15 — POSIX message queues
            MQ_OPEN    => crate::ipc::mqueue::sys_mq_open(arg0, arg1 as i32, arg2 as u32, arg3),
            MQ_CLOSE   => crate::ipc::mqueue::sys_mq_close(arg0 as i32),
            MQ_SEND    => crate::ipc::mqueue::sys_mq_send(arg0 as i32, arg1, arg2 as usize, arg3 as u32),
            MQ_RECEIVE => crate::ipc::mqueue::sys_mq_receive(arg0 as i32, arg1, arg2 as usize, arg3),
            MQ_UNLINK  => crate::ipc::mqueue::sys_mq_unlink(arg0),
            MQ_GETATTR => crate::ipc::mqueue::sys_mq_getattr(arg0 as i32, arg1),
            // Phase 16 — POSIX I/O multiplexing
            SELECT     => sys_select(arg0 as i32, arg1, arg2, arg3, arg4),
            // Phase 17 — Process model extensions
            UMASK            => sys_umask(arg0 as u32),
            SIGALTSTACK      => sys_sigaltstack(arg0, arg1),
            // Phase 18 — alarm + machine control
            ALARM            => sys_alarm(arg0),
            MACHINE_REBOOT   => sys_machine_reboot(),
            MACHINE_POWEROFF => sys_machine_poweroff(),
            // Phase 19 — POSIX UID/GID
            GETEUID  => sys_geteuid(),
            GETEGID  => sys_getegid(),
            SETUID   => sys_setuid(arg0 as u32),
            SETGID   => sys_setgid(arg0 as u32),
            SETEUID  => sys_seteuid(arg0 as u32),
            SETEGID  => sys_setegid(arg0 as u32),
            CHMOD    => sys_chmod(arg0 as *const u8, arg1 as u32),
            FCHMOD   => sys_fchmod(arg0 as i32, arg1 as u32),
            CHOWN    => sys_chown(arg0 as *const u8, arg1 as u32, arg2 as u32),
            FCHOWN   => sys_fchown(arg0 as i32, arg1 as u32, arg2 as u32),
            LCHOWN   => sys_chown(arg0 as *const u8, arg1 as u32, arg2 as u32), // no symlinks yet
            MOUNT      => sys_mount(arg0, arg1, arg2, arg3, arg4),
            GETMOUNTS  => sys_getmounts(arg0 as *mut u8, arg1 as usize),
            // musl/Linux ABI compatibility (numbers 115-161)
            SET_TID_ADDRESS  => posix_abi::sys_set_tid_address(arg0),
            SET_ROBUST_LIST  => posix_abi::sys_set_robust_list(arg0, arg1 as usize),
            GET_ROBUST_LIST  => posix_abi::sys_get_robust_list(arg0 as i32, arg1, arg2),
            EXIT_GROUP       => posix_abi::sys_exit_group(arg0 as i32),
            BRK              => posix_abi::sys_brk(arg0),
            OPENAT           => posix_abi::sys_openat(arg0 as i32, arg1 as *const u8, arg2 as i32, arg3 as u32),
            FSTATAT          => posix_abi::sys_fstatat(arg0 as i32, arg1 as *const u8, arg2 as *mut u64, arg3 as i32),
            UNLINKAT         => posix_abi::sys_unlinkat(arg0 as i32, arg1 as *const u8, arg2 as i32),
            MKDIRAT          => posix_abi::sys_mkdirat(arg0 as i32, arg1 as *const u8, arg2 as u32),
            FTRUNCATE        => posix_abi::sys_ftruncate(arg0 as i32, arg1),
            FDATASYNC        => posix_abi::sys_fdatasync(arg0 as i32),
            PIPE2            => posix_abi::sys_pipe2(arg0 as *mut i32, arg1 as i32),
            DUP3             => posix_abi::sys_dup3(arg0 as i32, arg1 as i32, arg2 as i32),
            FCNTL            => posix_abi::sys_fcntl(arg0 as i32, arg1 as i32, arg2),
            MPROTECT         => posix_abi::sys_mprotect(arg0, arg1 as usize, arg2 as i32),
            ACCESS           => posix_abi::sys_access(arg0 as *const u8, arg1 as i32),
            READLINK         => posix_abi::sys_readlink(arg0 as *const u8, arg1 as *mut u8, arg2 as usize),
            CLOCK_NANOSLEEP  => posix_abi::sys_clock_nanosleep(arg0 as i32, arg1 as i32, arg2 as *const u64, arg3 as *mut u64),
            CLOCK_GETRES     => posix_abi::sys_clock_getres(arg0 as i32, arg1 as *mut u64),
            WAITID           => posix_abi::sys_waitid(arg0 as i32, arg1 as i32, arg2 as *mut u8, arg3 as i32, arg4),
            TKILL            => posix_abi::sys_tkill(arg0 as i32, arg1 as i32),
            TGKILL           => posix_abi::sys_tgkill(arg0 as i32, arg1 as i32, arg2 as i32),
            MREMAP           => posix_abi::sys_mremap(arg0, arg1, arg2, arg3 as i32, arg4),
            MADVISE          => posix_abi::sys_madvise(arg0, arg1 as usize, arg2 as i32),
            MSYNC            => posix_abi::sys_msync(arg0, arg1 as usize, arg2 as i32),
            SYMLINK          => posix_abi::sys_symlink(arg0 as *const u8, arg1 as *const u8),
            LINK             => posix_abi::sys_link(arg0 as *const u8, arg1 as *const u8),
            READLINKAT       => posix_abi::sys_readlinkat(arg0 as i32, arg1 as *const u8, arg2 as *mut u8, arg3 as usize),
            FCHOWNAT         => posix_abi::sys_fchownat(arg0 as i32, arg1 as *const u8, arg2 as u32, arg3 as u32, arg4 as i32),
            FCHMODAT         => posix_abi::sys_fchmodat(arg0 as i32, arg1 as *const u8, arg2 as u32, arg3 as i32),
            FCHDIR           => posix_abi::sys_fchdir(arg0 as i32),
            STATX            => posix_abi::sys_statx(arg0 as i32, arg1 as *const u8, arg2 as i32, arg3 as u32, arg4 as *mut u8),
            READV            => posix_abi::sys_readv(arg0 as i32, arg1, arg2 as i32),
            WRITEV           => posix_abi::sys_writev(arg0 as i32, arg1, arg2 as i32),
            PREAD64          => posix_abi::sys_pread64(arg0 as i32, arg1 as *mut u8, arg2 as usize, arg3),
            PWRITE64         => posix_abi::sys_pwrite64(arg0 as i32, arg1 as *const u8, arg2 as usize, arg3),
            RENAMEAT         => posix_abi::sys_renameat(arg0 as i32, arg1 as *const u8, arg2 as i32, arg3 as *const u8),
            TIMES            => posix_abi::sys_times(arg0 as *mut u64),
            GETGROUPS        => posix_abi::sys_getgroups(arg0 as i32, arg1 as *mut u32),
            GETPGID          => posix_abi::sys_getpgid_syscall(arg0 as i32),
            CLOCK_SETTIME    => posix_abi::sys_clock_settime(arg0 as i32, arg1 as *const u64),
            TIMER_CREATE     => posix_abi::sys_timer_create(arg0 as i32, arg1, arg2 as *mut i32),
            TIMER_SETTIME    => posix_abi::sys_timer_settime(arg0 as i32, arg1 as i32, arg2, arg3),
            TIMER_GETTIME    => posix_abi::sys_timer_gettime(arg0 as i32, arg1),
            TIMER_DELETE     => posix_abi::sys_timer_delete(arg0 as i32),
            SETITIMER        => posix_abi::sys_setitimer(arg0 as i32, arg1, arg2),
            GETITIMER        => posix_abi::sys_getitimer(arg0 as i32, arg1),
            // SCM_RIGHTS support (v0.3+)
            SENDMSG          => crate::ipc::socket::sys_sendmsg(arg0 as i32, arg1, arg2 as i32),
            RECVMSG          => crate::ipc::socket::sys_recvmsg(arg0 as i32, arg1, arg2 as i32),
            _          => ENOSYS,
        }
    };

    // For SVC exceptions, ELR_EL1 already holds the address of the instruction
    // AFTER the SVC (ARM ARM DDI 0487 §D1.10.1 "Preferred exception return address").
    // Do NOT advance ELR — it is already the correct return address.
    unsafe {
        (*frame).x[0] = return_value as u64;
    }

    // Deliver any pending signals to the current process before returning to user.
    unsafe { signals::deliver_pending_signals(frame) };
}
