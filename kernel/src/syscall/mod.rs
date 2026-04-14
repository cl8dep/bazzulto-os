// syscall/mod.rs — System call dispatch and implementation.
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
}

use numbers::*;

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
            SPAWN         => sys_spawn(arg0 as *const u8, arg1 as usize, arg2),
            LIST          => sys_list(arg0 as *mut u8, arg1 as usize),
            WAIT          => sys_wait(arg0 as i32, arg1 as *mut i32),
            PIPE          => sys_pipe(arg0 as *mut i32),
            DUP           => sys_dup(arg0 as i32),
            DUP2          => sys_dup2(arg0 as i32, arg1 as i32),
            MMAP          => sys_mmap(arg0, arg1, arg2 as i32, arg3 as i32, arg4 as i32, arg5),
            MUNMAP        => sys_munmap(arg0, arg1),
            FORK          => sys_fork(frame),
            EXEC          => sys_exec(frame, arg0 as *const u8, arg1 as usize, arg2 as *const u8, arg3 as usize, arg4 as *const u8, arg5 as usize),
            GETPID        => sys_getpid(),
            GETPPID       => sys_getppid(),
            CLOCK_GETTIME => sys_clock_gettime(arg0 as i32, arg1 as *mut u64),
            NANOSLEEP     => sys_nanosleep(arg0 as *const u64, arg1 as *mut u64),
            SIGACTION     => sys_sigaction(arg0 as i32, arg1, arg2 as *mut u64, arg3 as u32),
            KILL          => sys_kill(arg0 as i32, arg1 as i32),
            SIGRETURN     => sys_sigreturn(frame),
            CREAT         => sys_creat(arg0 as *const u8, arg1 as usize, arg2 as u32),
            UNLINK        => sys_unlink(arg0 as *const u8, arg1 as usize),
            FSTAT         => sys_fstat(arg0 as *const u8, arg1 as usize, arg2 as *mut u64),
            SETFGPID      => sys_setfgpid(arg0 as i32),
            DISK_INFO     => sys_disk_info(arg0 as *mut u64),
            GETRANDOM     => sys_getrandom(arg0 as *mut u8, arg1 as usize),
            NICE          => sys_nice(arg0 as i32),
            GETPRIORITY   => sys_getpriority(),
            SETPRIORITY   => sys_setpriority(arg0 as i32),
            GETRLIMIT     => sys_getrlimit(arg0 as u32, arg1 as *mut u64),
            SETRLIMIT     => sys_setrlimit(arg0 as u32, arg1 as u64),
            GETPGRP       => sys_getpgrp(),
            SETPGID       => sys_setpgid(arg0 as i32, arg1 as i32),
            GETSID        => sys_getsid(arg0 as i32),
            SETSID        => sys_setsid(),
            TCGETPGRP     => sys_tcgetpgrp(arg0 as i32),
            TCSETPGRP     => sys_tcsetpgrp(arg0 as i32, arg1 as i32),
            // Phase 8
            UNAME         => sys_uname(arg0 as *mut u8),
            SYSINFO       => sys_sysinfo(arg0 as *mut u64),
            SIGPROCMASK   => sys_sigprocmask(arg0 as i32, arg1 as *const u64, arg2 as *mut u64),
            SIGPENDING    => sys_sigpending(arg0 as *mut u64),
            SIGSUSPEND    => sys_sigsuspend(frame, arg0 as u64),
            GETRUSAGE     => sys_getrusage(arg0 as i32, arg1 as *mut u64),
            PRCTL         => sys_prctl(arg0 as i32, arg1 as *const u8, arg2 as usize),
            GETTIMEOFDAY  => sys_gettimeofday(arg0 as *mut u64),
            POLL          => sys_poll(arg0 as *mut u64, arg1 as usize, arg2 as i32),
            GETUID        => sys_getuid(),
            GETGID        => sys_getgid(),
            // Phase 10 — Terminal
            IOCTL         => sys_ioctl(arg0 as i32, arg1 as u32, arg2),
            TCGETATTR     => sys_tcgetattr(arg0 as i32, arg1 as *mut u8),
            TCSETATTR     => sys_tcsetattr(arg0 as i32, arg1 as i32, arg2 as *const u8),
            // Phase 11 — Futex / threading
            FUTEX         => sys_futex(arg0, arg1 as i32, arg2 as u32, arg3),
            // Phase 12 — epoll
            EPOLL_CREATE1 => sys_epoll_create1(arg0 as i32),
            EPOLL_CTL     => sys_epoll_ctl(arg0 as i32, arg1 as i32, arg2 as i32, arg3),
            EPOLL_WAIT    => sys_epoll_wait(arg0 as i32, arg1, arg2 as i32, arg3 as i32),
            // Phase 13 — POSIX threads primitives
            CLONE         => sys_clone(frame, arg0, arg1, arg2),
            SET_TLS       => sys_set_tls(arg0),
            GETTID        => sys_gettid(),
            // Display
            FRAMEBUFFER_MAP => sys_framebuffer_map(arg0 as *mut u64),
            // Phase 9 — VFS syscalls
            CHDIR         => sys_chdir(arg0 as *const u8, arg1 as usize),
            GETCWD        => sys_getcwd(arg0 as *mut u8, arg1 as usize),
            MKDIR         => sys_mkdir(arg0 as *const u8, arg1 as usize, arg2 as u32),
            RMDIR         => sys_rmdir(arg0 as *const u8, arg1 as usize),
            RENAME        => sys_rename(arg0 as *const u8, arg1 as usize, arg2 as *const u8, arg3 as usize),
            GETDENTS64    => sys_getdents64(arg0 as i32, arg1 as *mut u8, arg2 as usize),
            TRUNCATE      => sys_truncate_fd(arg0 as i32, arg1 as u64),
            FSYNC         => sys_fsync(arg0 as i32),
            // Phase 14 — FIFOs and POSIX semaphores
            MKFIFO        => sys_mkfifo(arg0, arg1 as usize),
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
            MQ_OPEN    => crate::ipc::mqueue::sys_mq_open(arg0, arg1 as usize, arg2 as i32, arg3 as u32, arg4),
            MQ_CLOSE   => crate::ipc::mqueue::sys_mq_close(arg0 as i32),
            MQ_SEND    => crate::ipc::mqueue::sys_mq_send(arg0 as i32, arg1, arg2 as usize, arg3 as u32),
            MQ_RECEIVE => crate::ipc::mqueue::sys_mq_receive(arg0 as i32, arg1, arg2 as usize, arg3),
            MQ_UNLINK  => crate::ipc::mqueue::sys_mq_unlink(arg0, arg1 as usize),
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
            CHMOD    => sys_chmod(arg0, arg1 as usize, arg2 as u32),
            FCHMOD   => sys_fchmod(arg0 as i32, arg1 as u32),
            CHOWN    => sys_chown(arg0, arg1 as usize, arg2 as u32, arg3 as u32),
            FCHOWN   => sys_fchown(arg0 as i32, arg1 as u32, arg2 as u32),
            LCHOWN   => sys_chown(arg0, arg1 as usize, arg2 as u32, arg3 as u32), // no symlinks yet
            MOUNT      => sys_mount(arg0, arg1 as usize, arg2, arg3 as usize, arg4, arg5 as usize),
            GETMOUNTS  => sys_getmounts(arg0 as *mut u8, arg1 as usize),
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
    unsafe { deliver_pending_signals(frame) };
}

// ---------------------------------------------------------------------------
// Signal delivery before returning to user space
// ---------------------------------------------------------------------------

unsafe fn deliver_pending_signals(frame: *mut ExceptionFrame) {
    crate::scheduler::with_scheduler(|scheduler| {
        let current_pid = scheduler.current_pid();
        let signal_number = match scheduler.current_process() {
            Some(process) => process.take_pending_signal(),
            None => return,
        };

        let signal_number = match signal_number {
            Some(s) => s,
            None => return,
        };

        // DEBUG: log signal delivery.
        crate::drivers::uart::puts("[signal] delivering signum=");
        crate::drivers::uart::put_hex(signal_number as u64);
        crate::drivers::uart::puts(" to pid=");
        crate::drivers::uart::put_hex(current_pid.index as u64);
        crate::drivers::uart::puts("\r\n");

        // Look up the signal handler.
        let (action, trampoline_va) = match scheduler.current_process() {
            Some(process) => (
                process.signal_handlers[signal_number as usize],
                process.signal_trampoline_va(),
            ),
            None => return,
        };

        use crate::process::SignalAction;
        match action {
            SignalAction::Ignore => {}
            SignalAction::Default => {
                // Default action for most signals: terminate the process.
                // SIGCHLD and SIGURG: ignore by default.
                match signal_number {
                    17 | 23 => {} // SIGCHLD, SIGURG — ignore
                    _ => {
                        // Exit the process with signal number as exit code.
                        scheduler.exit(-(signal_number as i32));
                    }
                }
            }
            SignalAction::Handler { va: handler_va, on_stack } => {
                // Determine the stack pointer to use for signal delivery.
                //
                // If the handler was registered with SA_ONSTACK, the process has
                // an alternate stack configured, and we are not already executing
                // on that stack, switch to the top of the alternate stack.
                // Otherwise deliver on the current user stack.
                //
                // Reference: POSIX.1-2017 sigaltstack(2), sigaction(2) SA_ONSTACK.
                let base_sp = if on_stack {
                    // Read signal_stack and on_signal_stack state within the
                    // with_scheduler borrow already held above — we are already
                    // inside with_scheduler here, so access process fields directly
                    // by re-fetching from the same scheduler reference.
                    // NOTE: we are already inside `with_scheduler` in this closure.
                    // Accessing `process` again requires re-matching from `scheduler`.
                    // We do that by reading the fields we need before the match.
                    // The fields were already fetched via `action` above; here we
                    // need `signal_stack` and `on_signal_stack`.  Since we are in
                    // the same closure, fetch them inline.
                    //
                    // We can't call with_scheduler recursively, so read from the
                    // outer scheduler variable — but this closure already captures it.
                    // The outer with_scheduler closure uses a different binding below;
                    // we pull the process again from `scheduler` already in scope.
                    //
                    // The entire deliver_pending_signals is called from a single
                    // with_scheduler closure (line 437). We have `scheduler` in scope.
                    let use_alt = match scheduler.current_process() {
                        Some(p) => p.signal_stack.is_some() && !p.on_signal_stack,
                        None => false,
                    };
                    if use_alt {
                        if let Some(p) = scheduler.current_process_mut() {
                            p.on_signal_stack = true;
                            let ss = p.signal_stack.unwrap();
                            ss.base + ss.size as u64
                        } else {
                            (*frame).sp
                        }
                    } else {
                        (*frame).sp
                    }
                } else {
                    (*frame).sp
                };

                // Signal frame layout (grows downward from base_sp):
                //
                //   [base_sp - 32]  saved ELR_EL0  (pre-signal PC)
                //   [base_sp - 24]  saved SPSR_EL1 (pre-signal pstate)
                //   [base_sp - 16]  (alignment padding)
                //   [base_sp -  8]  (alignment padding)
                //   ← new sp (= base_sp - 32)
                //
                // sys_sigreturn restores ELR and SPSR from [sp+0] and [sp+8],
                // then advances sp by 32.
                //
                // Reference: ARM AAPCS64 §6.2 (stack must be 16-byte aligned at call).
                let new_sp = base_sp.wrapping_sub(32);
                core::ptr::write(new_sp as *mut u64, (*frame).elr);
                core::ptr::write((new_sp + 8) as *mut u64, (*frame).spsr);

                (*frame).sp = new_sp;
                // x0 = signal number (first argument to the handler).
                (*frame).x[0] = signal_number as u64;
                // x30 (link register) = signal trampoline.
                // The trampoline executes `svc #SIGRETURN` so that returning
                // from the handler lands in sys_sigreturn.
                (*frame).x[30] = trampoline_va;
                // ELR = handler entry point — eret resumes here.
                (*frame).elr = handler_va;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// sys_exit
// ---------------------------------------------------------------------------

unsafe fn sys_exit(exit_code: i32) -> i64 {
    // Grab the FD table Arc before entering the scheduler critical section.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    // Close all descriptors so pipe write-ends are released promptly.
    // We always close: this is the only place fds are released on exit.
    // The Arc count is at least 2 here (one in the process struct + the
    // clone above), so the old `== 1` guard was always false — a bug that
    // caused pipes to never receive EOF.
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        guard.close_all();
    }
    crate::scheduler::with_scheduler::<_, ()>(|scheduler| {
        scheduler.exit(exit_code);
    });
    // Never reached.
    #[allow(unreachable_code)]
    0
}

// ---------------------------------------------------------------------------
// sys_write
// ---------------------------------------------------------------------------

unsafe fn sys_write(fd: i32, buffer_ptr: *const u8, length: usize) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(buffer_ptr as u64, length) {
        // POSIX.1-2017 write(2): EFAULT if buf is outside the accessible address space.
        return EFAULT;
    }
    let source_slice = core::slice::from_raw_parts(buffer_ptr, length);

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EBADF,
    };
    let mut guard = fd_table_arc.lock();
    let result = if let Some(descriptor) = guard.get_mut(fd as usize) {
        descriptor.write(source_slice)
    } else {
        return EBADF;
    };
    drop(guard);

    // i64::MIN is the sentinel returned by FileDescriptor::write when the
    // read end of a pipe is closed.  POSIX.1-2017 write(2) requires:
    //   1. SIGPIPE is generated for the process.
    //   2. -1 is returned with errno set to EPIPE.
    // If SIGPIPE is set to SIG_IGN the signal is not delivered but EPIPE is
    // still returned — the signal handler check inside deliver_pending_signals
    // handles the SIG_IGN case automatically.
    if result == i64::MIN {
        const SIGPIPE: u8 = 13;
        crate::scheduler::with_scheduler(|scheduler| {
            let pid = scheduler.current_pid();
            scheduler.send_signal_to(pid, SIGPIPE);
        });
        return EPIPE;
    }

    result
}

// ---------------------------------------------------------------------------
// sys_read
// ---------------------------------------------------------------------------

unsafe fn sys_read(fd: i32, buffer_ptr: *mut u8, length: usize) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(buffer_ptr as u64, length) {
        // POSIX.1-2017 read(2): EFAULT if buf is outside the accessible address space.
        return EFAULT;
    }
    let destination_slice = core::slice::from_raw_parts_mut(buffer_ptr, length);

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EBADF,
    };
    let mut guard = fd_table_arc.lock();
    let is_nonblock = crate::fs::vfs::FileDescriptorTable::mask_test(&guard.nonblock_mask, fd as usize);

    // If reading from the TTY (fd == stdin or any TTY fd), wire up the
    // echo sink before acquiring the mutable descriptor reference so we
    // avoid a simultaneous borrow of `guard`.
    //
    // Strategy: check fd 1 (stdout) now while we have a shared borrow,
    // grab its raw PipeBuffer pointer if it is a write-end pipe, then do
    // the mutable get_mut below.  The PipeBuffer is heap-allocated and
    // lives as long as any handle to the pipe exists, so the raw pointer
    // is stable for the duration of this syscall.
    let is_tty_read = matches!(guard.get(fd as usize),
        Some(crate::fs::vfs::FileDescriptor::Tty));
    let echo_buf: *mut crate::fs::pipe::PipeBuffer = if is_tty_read {
        if let Some(crate::fs::vfs::FileDescriptor::Pipe(stdout_handle)) = guard.get(1) {
            use crate::fs::pipe::PipeEnd;
            if stdout_handle.end() == PipeEnd::WriteEnd {
                stdout_handle.buffer_mut() as *mut _
            } else {
                core::ptr::null_mut()
            }
        } else {
            core::ptr::null_mut()
        }
    } else {
        core::ptr::null_mut()
    };

    if is_tty_read {
        crate::drivers::tty::tty_set_echo_sink(echo_buf);
    }

    let result = if let Some(descriptor) = guard.get_mut(fd as usize) {
        // Check O_NONBLOCK: if set and the descriptor is a pipe, attempt a
        // non-blocking read and return EAGAIN instead of blocking.
        if is_nonblock {
            if let crate::fs::vfs::FileDescriptor::Pipe(handle) = descriptor {
                use crate::fs::pipe::PipeEnd;
                if handle.end() != PipeEnd::ReadEnd {
                    return EAGAIN;
                }
                let buf = handle.buffer_mut();
                if buf.available_to_read() == 0 {
                    if buf.is_write_closed() {
                        return 0; // EOF
                    }
                    return EAGAIN;
                }
                return buf.read_bytes(destination_slice) as i64;
            }
        }
        descriptor.read(destination_slice)
    } else {
        EBADF
    };

    if is_tty_read {
        crate::drivers::tty::tty_clear_echo_sink();
    }

    result
}

// ---------------------------------------------------------------------------
// sys_yield
// ---------------------------------------------------------------------------

unsafe fn sys_yield() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.schedule();
    });
    0
}

// ---------------------------------------------------------------------------
// sys_open
// ---------------------------------------------------------------------------

unsafe fn sys_open(name_ptr: *const u8, name_length: usize, flags: i32, mode: u32) -> i64 {
    if name_ptr.is_null() {
        return EINVAL;
    }
    if name_ptr as u64 >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name_raw = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    // Resolve relative paths (e.g. "./foo", "foo") to absolute before dispatch.
    // Scheme paths ("//proc:...") are always absolute and left unchanged.
    let name_abs_owned;
    let name = if name_raw.starts_with("//") {
        name_raw
    } else {
        name_abs_owned = resolve_to_absolute(name_raw);
        name_abs_owned.as_str()
    };

    // Binary Permission Model — access permission check.
    //
    // Check before any inode lookup to prevent path enumeration: a denied
    // path returns EACCES regardless of whether the inode exists.
    //
    // Impossible namespaces are always denied.
    // If granted_permissions is non-empty, the path must match at least one
    // pattern.  An empty set means Tier-4 transitional mode (bypass).
    //
    // Only canonical `//scheme:` paths are checked here — POSIX `/dev/`, `/proc/`
    // paths go through a separate device dispatch before reaching the VFS.
    //
    // Reference: docs/features/Binary Permission Model.md §vfs_open check.
    if name.starts_with("//") {
        if crate::permission::is_impossible_namespace(name) {
            return EPERM;
        }
        let access_denied = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| !crate::permission::permission_allows(&p.granted_permissions, name))
                .unwrap_or(false)
        });
        if access_denied {
            return EACCES;
        }
    }

    // Dispatch by path scheme.
    // NOTE: FAT32 is now mounted at /mnt via the VFS mount table — no special
    // case needed.  Paths starting with /mnt/ are resolved by vfs_resolve()
    // which finds the Fat32DirInode mounted there.
    let descriptor = if name.starts_with("//proc:") {
        // Procfs virtual file.
        match crate::fs::procfs::procfs_open(name) {
            Some(snapshot) => crate::fs::vfs::FileDescriptor::ProcFile(snapshot),
            None => return ENOENT,
        }
    } else if name == "/dev/ptmx" {
        // PTY master: allocate a new PTY pair and return the master fd.
        let pty_index = match crate::drivers::pty::pty_allocate() {
            Some(index) => index,
            None => return EINVAL, // no PTY slots available
        };
        let master_inode = crate::drivers::pty::pty_master_inode(pty_index);
        crate::fs::vfs::FileDescriptor::InoFile { inode: master_inode, position: 0 }
    } else if let Some(pts_name) = name.strip_prefix("/dev/pts/") {
        // PTY slave: parse the index and return the slave fd.
        let pty_index: usize = match pts_name.parse() {
            Ok(index) => index,
            Err(_) => return ENOENT,
        };
        if pty_index >= crate::drivers::pty::PTY_MAX {
            return ENOENT;
        }
        let slave_inode = crate::drivers::pty::pty_slave_inode(pty_index);
        crate::fs::vfs::FileDescriptor::InoFile { inode: slave_inode, position: 0 }
    } else if name.starts_with('/') {
        // Absolute path: try ramfs first (service files, ELFs embedded at build
        // time), then fall back to VFS (tmpfs / devfs).
        if let Some(data) = crate::fs::ramfs_find(name) {
            crate::fs::vfs::FileDescriptor::RamFsFile { data, position: 0 }
        } else {
            let cwd = crate::scheduler::with_scheduler(|scheduler| {
                scheduler.current_process().and_then(|p| p.cwd.clone())
            });
            match crate::fs::vfs_resolve(name, cwd.as_ref()) {
                Ok(inode) => {
                    if flags & O_EXCL != 0 && flags & O_CREAT != 0 {
                        return EEXIST;
                    }
                    if flags & O_TRUNC != 0 {
                        let _ = inode.truncate(0);
                    }
                    crate::fs::vfs::FileDescriptor::InoFile { inode, position: 0 }
                }
                Err(_) if flags & O_CREAT != 0 => {
                    // File does not exist and O_CREAT is set — create it.
                    let umask = crate::scheduler::with_scheduler(|scheduler| {
                        scheduler.current_process().map(|p| p.umask).unwrap_or(0o022)
                    });
                    // Preserve file type bits (upper bits) from mode, apply umask to
                    // permission bits only.  Callers typically pass 0o666 for files.
                    let effective_mode = (0o100000u64)
                        | (((mode as u64) & 0o777) & !((umask as u64) & 0o777));
                    let (parent, file_name) = match crate::fs::vfs_resolve_parent(name) {
                        Ok(pair) => pair,
                        Err(_) => return ENOENT,
                    };
                    let inode = match parent.create(&file_name) {
                        Ok(inode) => inode,
                        Err(e) => return e.to_errno(),
                    };
                    let _ = inode.set_mode(effective_mode);
                    crate::fs::vfs::FileDescriptor::InoFile { inode, position: 0 }
                }
                Err(_) => return ENOENT,
            }
        }
    } else {
        // Bare name: try ramfs first, then VFS relative to cwd.
        if let Some(data) = crate::fs::ramfs_find(name) {
            crate::fs::vfs::FileDescriptor::RamFsFile { data, position: 0 }
        } else {
            let cwd = crate::scheduler::with_scheduler(|scheduler| {
                scheduler.current_process().and_then(|p| p.cwd.clone())
            });
            match crate::fs::vfs_resolve(name, cwd.as_ref()) {
                Ok(inode) => {
                    if flags & O_EXCL != 0 && flags & O_CREAT != 0 {
                        return EEXIST;
                    }
                    if flags & O_TRUNC != 0 {
                        let _ = inode.truncate(0);
                    }
                    crate::fs::vfs::FileDescriptor::InoFile { inode, position: 0 }
                }
                Err(_) if flags & O_CREAT != 0 => {
                    let umask = crate::scheduler::with_scheduler(|scheduler| {
                        scheduler.current_process().map(|p| p.umask).unwrap_or(0o022)
                    });
                    let effective_mode = (0o100000u64)
                        | (((mode as u64) & 0o777) & !((umask as u64) & 0o777));
                    let (parent, file_name) = match crate::fs::vfs_resolve_parent(name) {
                        Ok(pair) => pair,
                        Err(_) => return ENOENT,
                    };
                    let inode = match parent.create(&file_name) {
                        Ok(inode) => inode,
                        Err(e) => return e.to_errno(),
                    };
                    let _ = inode.set_mode(effective_mode);
                    crate::fs::vfs::FileDescriptor::InoFile { inode, position: 0 }
                }
                Err(_) => return ENOENT,
            }
        }
    };

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let fd = guard.install(descriptor);
    if fd < 0 {
        return EMFILE as i64;
    }
    if flags & O_CLOEXEC != 0 {
        crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.cloexec_mask, fd as usize);
    }
    if flags & O_NONBLOCK != 0 {
        crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.nonblock_mask, fd as usize);
    }
    fd as i64
}

// ---------------------------------------------------------------------------
// sys_close
// ---------------------------------------------------------------------------

unsafe fn sys_close(fd: i32) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    if guard.close(fd as usize) {
        crate::fs::vfs::FileDescriptorTable::mask_clear(&mut guard.cloexec_mask, fd as usize);
        crate::fs::vfs::FileDescriptorTable::mask_clear(&mut guard.nonblock_mask, fd as usize);
        0
    } else {
        EBADF
    }
}

// ---------------------------------------------------------------------------
// sys_seek
// ---------------------------------------------------------------------------

unsafe fn sys_seek(fd: i32, offset: i64, whence: i32) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    if let Some(descriptor) = guard.get_mut(fd as usize) {
        descriptor.seek(offset, whence)
    } else {
        EBADF
    }
}

// ---------------------------------------------------------------------------
// sys_spawn — load and execute a ramfs binary as a new process
// ---------------------------------------------------------------------------

unsafe fn sys_spawn(name_ptr: *const u8, name_length: usize, capability_mask: u64) -> i64 {
    if name_ptr.is_null() {
        return EINVAL;
    }
    if name_ptr as u64 >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Try VFS first (FAT32 disk), fall back to legacy ramfs for built-ins.
    // This mirrors the pattern used by sys_exec.
    // Also collect cwd_path so the child inherits the working directory.
    let (cwd, parent_cwd_path) = crate::scheduler::with_scheduler(|scheduler| {
        let cwd      = scheduler.current_process().and_then(|p| p.cwd.clone());
        let cwd_path = scheduler.current_process()
            .map(|p| p.cwd_path.clone())
            .unwrap_or_else(|| alloc::string::String::from("/"));
        (cwd, cwd_path)
    });
    let vfs_inode = crate::fs::vfs_resolve(name, cwd.as_ref()).ok();
    let owned_elf: alloc::vec::Vec<u8>;
    let elf_data: &[u8] = if let Some(ref inode) = vfs_inode {
        let size = inode.stat().size as usize;
        owned_elf = {
            let mut buf = alloc::vec![0u8; size];
            let _ = inode.read_at(0, &mut buf);
            buf
        };
        &owned_elf
    } else if let Some(data) = crate::fs::ramfs_find(name) {
        data
    } else {
        return ENOENT;
    };

    // Collect the parent's environ so the child inherits it on the initial stack.
    // We build a Vec<Vec<u8>> of "KEY=VALUE" byte strings first, then build
    // &[&[u8]] slices for load_elf.
    let parent_environ: alloc::vec::Vec<alloc::vec::Vec<u8>> =
        crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => p.environ.iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect(),
                None => alloc::vec::Vec::new(),
            }
        });
    let envp_slices: alloc::vec::Vec<&[u8]> =
        parent_environ.iter().map(|v| v.as_slice()).collect();

    // Load the ELF into a new address space.
    let loaded = crate::memory::with_physical_allocator(|phys| {
        crate::loader::load_elf(elf_data, phys, &[], &envp_slices)
    });

    let loaded = match loaded {
        Ok(image) => image,
        Err(_) => return ENOMEM,
    };
    // Box the page table outside of with_physical_allocator to avoid
    // re-entrant access to the physical allocator via GlobalAlloc.
    let loaded_page_table = alloc::boxed::Box::new(loaded.page_table);

    // Create the new process.
    let child_pid = crate::scheduler::with_scheduler(|scheduler| {
        let parent_pid = scheduler.current_pid();

        // Capability check: if the caller is requesting capabilities, it must
        // hold CAP_SETCAP.  It may only grant capabilities it already holds.
        if capability_mask != 0 {
            let parent_caps = scheduler.current_process()
                .map(|p| p.capabilities)
                .unwrap_or(0);

            if parent_caps & crate::process::CAP_SETCAP == 0 {
                return None; // caller lacks CAP_SETCAP → EPERM
            }
            // Cannot grant capabilities not held by the parent.
            if capability_mask & !parent_caps != 0 {
                return None; // EPERM
            }
        }

        // Clone the parent's current fd table so the child inherits open fds
        // (including any pipe ends that bzinit redirected via dup2 before spawn).
        let parent_fd_table_clone = scheduler.current_process()
            .map(|p| {
                let guard = p.file_descriptor_table.lock();
                guard.clone_for_fork()
            });

        let child_pid = match scheduler.create_process(Some(parent_pid)) {
            Some(pid) => pid,
            None => return None,
        };

        let child = match scheduler.process_mut(child_pid) {
            Some(process) => process,
            None => return None,
        };

        // Install the new page table.
        child.page_table = Some(loaded_page_table);
        // Register the demand-paged stack region.
        child.mmap_regions.push(crate::process::MmapRegion {
            base:   loaded.stack_demand_base,
            length: loaded.stack_demand_top - loaded.stack_demand_base,
            demand: true,
            backing: crate::process::MmapBacking::Anonymous,
        });
        // Grant requested capabilities (already validated above).
        child.capabilities = capability_mask;
        // Inherit the parent's environment.
        child.environ = parent_environ.iter()
            .filter_map(|v| core::str::from_utf8(v).ok().map(|s| alloc::string::String::from(s)))
            .collect();

        // Inherit the parent's working directory.
        child.cwd      = cwd.clone();
        child.cwd_path = parent_cwd_path.clone();

        // Replace the fresh fd table with the parent's clone (inheriting pipes).
        if let Some(fd_clone) = parent_fd_table_clone {
            *child.file_descriptor_table.lock() = fd_clone;
        }

        // Build the initial ExceptionFrame on the child's kernel stack.
        let frame_ptr = (child.kernel_stack.top as usize
            - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
        core::ptr::write_bytes(frame_ptr as *mut u8, 0, core::mem::size_of::<ExceptionFrame>());
        // ELR_EL1 = entry point, SPSR_EL1 = EL0 (0x0), SP_EL0 = user stack.
        (*frame_ptr).elr = loaded.entry_point;
        (*frame_ptr).spsr = 0; // EL0 state
        (*frame_ptr).sp = loaded.initial_stack_pointer;
        crate::uart::puts("[spawn] sp=");
        crate::uart::put_hex(loaded.initial_stack_pointer);
        crate::uart::puts(" demand=[");
        crate::uart::put_hex(loaded.stack_demand_base);
        crate::uart::puts(", ");
        crate::uart::put_hex(loaded.stack_demand_top);
        crate::uart::puts(")\r\n");
        // AArch64 SYSV ABI: x0 = argc, x1 = argv[] VA, x2 = envp[] VA.
        (*frame_ptr).x[0] = loaded.argc as u64;
        (*frame_ptr).x[1] = loaded.argv_va;
        (*frame_ptr).x[2] = loaded.envp_va;

        child.cpu_context.stack_pointer = frame_ptr as u64;
        child.cpu_context.link_register = crate::process::process_entry_trampoline_el0 as *const () as u64;

        // Mark the new process as foreground.
        child.is_foreground = true;

        scheduler.make_ready(child_pid);
        Some(child_pid)
    });

    match child_pid {
        Some(pid) => pid.index as i64,
        None => ENOMEM,
    }
}

// ---------------------------------------------------------------------------
// sys_list — list ramfs files into a user buffer
// ---------------------------------------------------------------------------

unsafe fn sys_list(buffer_ptr: *mut u8, buffer_length: usize) -> i64 {
    if buffer_ptr.is_null() {
        return EINVAL;
    }
    if buffer_ptr as u64 >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let destination = core::slice::from_raw_parts_mut(buffer_ptr, buffer_length);
    let mut written = 0usize;

    crate::fs::ramfs_list(|name| {
        let name_bytes = name.as_bytes();
        if written + name_bytes.len() + 1 <= buffer_length {
            destination[written..written + name_bytes.len()].copy_from_slice(name_bytes);
            written += name_bytes.len();
            destination[written] = b'\n';
            written += 1;
        }
    });

    written as i64
}

// ---------------------------------------------------------------------------
// sys_wait — wait for a child process to exit
// ---------------------------------------------------------------------------

unsafe fn sys_wait(pid_arg: i32, status_ptr: *mut i32) -> i64 {
    let for_pid = if pid_arg < 0 {
        None // wait for any child
    } else {
        Some(crate::process::Pid::new(pid_arg as u16, 1))
    };

    // POSIX.1-2017 wait(2): if the calling process has no existing unwaited-for
    // child processes, return ECHILD immediately rather than blocking forever.
    let has_any_children = crate::scheduler::with_scheduler(|scheduler| {
        let current_pid = scheduler.current_pid();
        scheduler.has_children(current_pid)
    });
    if !has_any_children {
        return ECHILD;
    }

    loop {
        let result = crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            scheduler.reap(current_pid, for_pid)
        });

        if let Some((reaped_pid, exit_code)) = result {
            if !status_ptr.is_null() && (status_ptr as u64) < crate::process::USER_ADDR_LIMIT {
                // POSIX.1-2017 waitpid(2): encode exit status so that the
                // standard WIFEXITED(status) and WEXITSTATUS(status) macros
                // work correctly.  Normal termination encodes exit_code in
                // bits [15:8] with bits [7:0] == 0.
                // Reference: POSIX.1-2017 §2.13, sys/wait.h macros.
                *status_ptr = (exit_code & 0xFF) << 8;
            }
            return reaped_pid.index as i64;
        }

        // No zombie child yet — block until one exits.
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process_mut() {
                process.state = crate::process::ProcessState::Waiting { for_pid };
            }
            scheduler.schedule_no_requeue();
        });
    }
}

// ---------------------------------------------------------------------------
// sys_pipe — create a pipe
// ---------------------------------------------------------------------------

unsafe fn sys_pipe(fd_pair_ptr: *mut i32) -> i64 {
    if fd_pair_ptr.is_null() {
        return EINVAL;
    }
    // Validate that both words of the int[2] are within user address space.
    // A single bounds check on `fd_pair_ptr` alone is insufficient — a pointer
    // near USER_ADDR_LIMIT could cause the second write to go out of bounds.
    if !validate_user_pointer(fd_pair_ptr as u64, 2 * core::mem::size_of::<i32>()) {
        return EFAULT;
    }

    let (read_handle, write_handle) = crate::fs::pipe::pipe_create();
    let read_descriptor = crate::fs::vfs::FileDescriptor::Pipe(read_handle);
    let write_descriptor = crate::fs::vfs::FileDescriptor::Pipe(write_handle);

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let read_fd = guard.install(read_descriptor);
    if read_fd < 0 {
        return EMFILE as i64;
    }
    let write_fd = guard.install(write_descriptor);
    if write_fd < 0 {
        guard.close(read_fd as usize);
        return EMFILE as i64;
    }
    *fd_pair_ptr = read_fd;
    *fd_pair_ptr.add(1) = write_fd;
    0
}

// ---------------------------------------------------------------------------
// sys_dup
// ---------------------------------------------------------------------------

unsafe fn sys_dup(source_fd: i32) -> i64 {
    if source_fd < 0 {
        return EBADF;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let new_fd = guard.dup(source_fd as usize);
    if new_fd < 0 { EBADF } else { new_fd as i64 }
}

// ---------------------------------------------------------------------------
// sys_dup2
// ---------------------------------------------------------------------------

unsafe fn sys_dup2(source_fd: i32, destination_fd: i32) -> i64 {
    if source_fd < 0 || destination_fd < 0 {
        return EBADF;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let result = guard.dup2(source_fd as usize, destination_fd as usize);
    if result < 0 { EBADF } else { result as i64 }
}

// ---------------------------------------------------------------------------
// sys_mmap — memory mapping (anonymous and file-backed)
//
// ABI: mmap(addr, length, prot, flags, fd, offset)
//   addr:   hint (ignored; we use a bump pointer).
//   length: bytes to map (rounded up to a page boundary).
//   prot:   PROT_READ / PROT_WRITE / PROT_EXEC (stored but not enforced yet).
//   flags:  MAP_ANONYMOUS, MAP_PRIVATE, MAP_SHARED, MAP_FIXED (MAP_FIXED ignored).
//   fd:     file descriptor for file-backed mappings; -1 for anonymous.
//   offset: byte offset into the file (must be page-aligned for file-backed).
//
// Supported combinations:
//   MAP_ANONYMOUS | MAP_PRIVATE  — zero-filled anonymous (demand).
//   MAP_ANONYMOUS | MAP_SHARED   — anonymous shared (registered in SharedRegionTable).
//   fd >= 0 | MAP_PRIVATE        — file-backed CoW (demand, reads from inode on fault).
//   fd >= 0 | MAP_SHARED         — stubbed: treated as MAP_PRIVATE for now (post-v1.0).
//
// Reference: POSIX.1-2017 mmap(2).
// ---------------------------------------------------------------------------

unsafe fn sys_mmap(
    _addr: u64,
    length: u64,
    _prot: i32,
    flags: i32,
    fd: i32,
    offset: u64,
) -> i64 {
    if length == 0 {
        return EINVAL;
    }

    let page_size = crate::memory::physical::read_page_size();
    let pages = ((length + page_size - 1) / page_size) as usize;

    let is_anonymous = (flags & MAP_ANONYMOUS != 0) || fd < 0;
    let is_shared_anonymous = (flags & MAP_SHARED != 0) && is_anonymous;

    // --- File-backed MAP_PRIVATE ---
    // Resolve the inode now (outside the scheduler lock) so we can store an
    // Arc<dyn Inode> in the MmapRegion backing.
    let file_backing: Option<alloc::sync::Arc<dyn crate::fs::Inode>> = if !is_anonymous {
        // Validate fd and offset alignment.
        if offset % page_size != 0 {
            return EINVAL;
        }
        let inode = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process().and_then(|p| {
                let table = p.file_descriptor_table.lock();
                table.get(fd as usize).and_then(|desc| {
                    match desc {
                        crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                            Some(alloc::sync::Arc::clone(inode))
                        }
                        _ => None,
                    }
                })
            })
        });
        match inode {
            Some(inode) => Some(inode),
            None => return EBADF,
        }
    } else {
        None
    };

    // Allocate VA space via bump pointer; all regions are demand-paged.
    let base_va_opt = crate::scheduler::with_scheduler(|scheduler| {
        let process = scheduler.current_process_mut()?;
        if process.mmap_regions.len() >= crate::process::MMAP_MAX_REGIONS {
            return None;
        }
        let base = process.mmap_next_va;
        let region_length = pages as u64 * page_size;
        process.mmap_next_va = base + region_length;

        let backing = if let Some(ref inode) = file_backing {
            crate::process::MmapBacking::File {
                inode: alloc::sync::Arc::clone(inode),
                file_offset: offset,
            }
        } else {
            crate::process::MmapBacking::Anonymous
        };

        process.mmap_regions.push(crate::process::MmapRegion {
            base,
            length: region_length,
            demand: true,
            backing,
        });

        Some(base)
    });

    let base_va = match base_va_opt {
        Some(va) => va,
        None => return ENOMEM,
    };

    if is_shared_anonymous {
        // Register the region so fork() will map it shared rather than CoW.
        let table = &mut *SHARED_REGION_TABLE.0.get();
        table.insert(base_va, SharedRegion {
            phys_base: 0, // tracked by page table; placeholder
            page_count: pages,
            reference_count: 1,
        });
    }

    base_va as i64
}

// ---------------------------------------------------------------------------
// sys_munmap
// ---------------------------------------------------------------------------

unsafe fn sys_munmap(addr: u64, length: u64) -> i64 {
    if addr == 0 || length == 0 {
        return EINVAL;
    }

    let page_size = crate::memory::physical::read_page_size();

    // POSIX.1-2017 munmap(2): "The addr argument shall be a multiple of the
    // page size as returned by sysconf(_SC_PAGESIZE)."
    // Linux returns EINVAL for a non-page-aligned addr.
    if addr % page_size != 0 {
        return EINVAL;
    }
    let pages = ((length + page_size - 1) / page_size) as usize;

    crate::memory::with_physical_allocator(|phys| {
        crate::scheduler::with_scheduler(|scheduler| {
            if scheduler.munmap_for_current(addr, pages, page_size, phys) {
                0
            } else {
                EINVAL
            }
        })
    })
}

// ---------------------------------------------------------------------------
// sys_fork
// ---------------------------------------------------------------------------

unsafe fn sys_fork(frame: *mut ExceptionFrame) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.fork(frame) {
            Ok(child_pid) => child_pid.index as i64,
            // POSIX.1-2017 fork(2): EAGAIN if process/resource limit reached,
            // ENOMEM if insufficient memory.  InternalError is an OOM-class
            // failure, not a "process not found" situation, so ESRCH is wrong.
            Err(crate::scheduler::ForkError::OutOfPids) => EAGAIN,
            Err(crate::scheduler::ForkError::OutOfMemory) => ENOMEM,
            Err(crate::scheduler::ForkError::InternalError) => ENOMEM,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_exec — replace current process image with a VFS or ramfs binary
// ---------------------------------------------------------------------------

/// Parse a null-separated flat byte buffer into a fixed-size slice array.
///
/// Format: `"entry0\0entry1\0entry2\0"` — each entry ends at a NUL byte.
/// Entries that are not valid UTF-8 are silently skipped.
/// At most `N` entries are stored; excess entries are silently dropped.
///
/// Returns the number of entries written into `out`.
unsafe fn parse_flat_strings<'a, const N: usize>(
    ptr: *const u8,
    length: usize,
    out: &mut [&'a [u8]; N],
) -> usize {
    let mut count = 0usize;
    if ptr.is_null() || length == 0 || (ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return 0;
    }
    let flat = core::slice::from_raw_parts(ptr, length);
    let mut start = 0usize;
    for (index, &byte) in flat.iter().enumerate() {
        if byte == 0 {
            if index > start && count < N {
                let slice = &flat[start..index];
                if core::str::from_utf8(slice).is_ok() {
                    out[count] = slice;
                    count += 1;
                }
            }
            start = index + 1;
        }
    }
    count
}

unsafe fn sys_exec(
    frame: *mut ExceptionFrame,
    name_ptr: *const u8,
    name_length: usize,
    argv_ptr: *const u8,
    argv_len: usize,
    envp_ptr: *const u8,
    envp_len: usize,
) -> i64 {
    if name_ptr.is_null() || (name_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Sanitize argv_ptr and envp_ptr: callers that do not pass these arguments
    // (e.g. raw_exec with 4 args) leave x4/x5 as scratch-register garbage.
    // Any pointer below the first mapped user page (0x1000) is invalid; treat it
    // as a null / empty argument list to avoid a kernel data abort.
    // Reference: AArch64 VA layout — addresses < 0x1000 are always unmapped.
    let (argv_ptr, argv_len) = if (argv_ptr as u64) < 0x1000 {
        (core::ptr::null(), 0usize)
    } else {
        (argv_ptr, argv_len)
    };
    let (envp_ptr, envp_len) = if (envp_ptr as u64) < 0x1000 {
        (core::ptr::null(), 0usize)
    } else {
        (envp_ptr, envp_len)
    };

    // Resolve via VFS first; fall back to legacy ramfs.
    let vfs_inode = crate::fs::vfs_resolve(name, None).ok();
    let ramfs_data = if vfs_inode.is_none() { crate::fs::ramfs_find(name) } else { None };

    // INODE_KERNEL_EXEC_ONLY check: reject userspace exec of kernel-only binaries.
    //
    // vfs_mark_kernel_exec_only() is called during boot for /system/bin/bzinit.
    // Any subsequent userspace exec() of that path returns EPERM.
    //
    // Reference: docs/features/Binary Permission Model.md §INODE_KERNEL_EXEC_ONLY.
    if crate::fs::vfs_is_kernel_exec_only(name) {
        return EPERM;
    }

    // We need owned data for the VFS path because the inode may be backed by a
    // Vec<u8> whose lifetime is tied to the inode Arc — we copy it to avoid
    // borrowing through the scheduler lock.
    let owned_elf: alloc::vec::Vec<u8>;
    let elf_data: &[u8] = if let Some(ref inode) = vfs_inode {
        let size = inode.stat().size as usize;
        owned_elf = {
            let mut buf = alloc::vec![0u8; size];
            let _ = inode.read_at(0, &mut buf);
            buf
        };
        &owned_elf
    } else if let Some(data) = ramfs_data {
        data
    } else {
        return ENOENT;
    };

    // Binary Permission Model — tier dispatch at exec time.
    //
    // Tier 1: system binary → full trust (wildcard permissions).
    // Tier 4: no .bazzulto_permissions section → inherit from parent + warn.
    // Tier 2/3: section present → permissiond handles it (post-v1.0); we do
    //           not touch the sets and leave them as-is.
    //
    // Reference: docs/features/Binary Permission Model.md §Tier Dispatch.
    let exec_permission_tier_result = {
        let has_perm_section = crate::permission::elf_has_bazzulto_permissions_section(elf_data);
        let (parent_perms, parent_actions) = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| (p.granted_permissions.clone(), p.granted_actions.clone()))
                .unwrap_or_default()
        });
        crate::permission::resolve_exec_permissions(
            name,
            has_perm_section,
            &parent_perms,
            &parent_actions,
            name,
        )
    };

    // Parse argv (cap at 64 entries to bound stack usage).
    let mut argv_slices: [&[u8]; 64] = [&[]; 64];
    let argv_count = parse_flat_strings(argv_ptr, argv_len, &mut argv_slices);
    let argv = &argv_slices[..argv_count];

    // Parse envp (cap at 128 entries).
    let mut envp_slices: [&[u8]; 128] = [&[]; 128];
    let envp_count = parse_flat_strings(envp_ptr, envp_len, &mut envp_slices);
    let envp = &envp_slices[..envp_count];

    let loaded = crate::memory::with_physical_allocator(|phys| {
        crate::loader::load_elf(elf_data, phys, argv, envp)
    });

    let loaded = match loaded {
        Ok(image) => image,
        Err(crate::loader::LoaderError::NotAnElf)
        | Err(crate::loader::LoaderError::UnsupportedFormat)
        | Err(crate::loader::LoaderError::NotExecutable)
        | Err(crate::loader::LoaderError::UnalignedSegment)
        | Err(crate::loader::LoaderError::Truncated) => {
            // POSIX.1-2017 exec(2) §ERRORS: ENOEXEC if the file has the
            // appropriate access permission but an unrecognised format.
            return ENOEXEC;
        }
        Err(_) => return ENOMEM,
    };
    // Box the page table outside with_physical_allocator to avoid re-entrant
    // access to the physical allocator via GlobalAlloc.
    let loaded_page_table = alloc::boxed::Box::new(loaded.page_table);

    // Close cloexec FDs before replacing the address space.
    // Do this outside the scheduler lock to avoid holding two locks simultaneously.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        // Iterate all 16 words of the cloexec bitmask and close every marked fd.
        for word_index in 0..16usize {
            let mut word = guard.cloexec_mask[word_index];
            while word != 0 {
                let bit_pos = word.trailing_zeros() as usize;
                let fd_number = word_index * 64 + bit_pos;
                guard.close(fd_number);
                word &= word - 1;
            }
        }
        guard.cloexec_mask = [0u64; 16];
        guard.nonblock_mask = [0u64; 16];
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {

            // Replace address space.
            process.page_table = Some(loaded_page_table);
            process.mmap_next_va = crate::process::MMAP_USER_BASE;
            process.mmap_regions.clear();
            // Register the demand-paged stack region for the new image.
            process.mmap_regions.push(crate::process::MmapRegion {
                base:   loaded.stack_demand_base,
                length: loaded.stack_demand_top - loaded.stack_demand_base,
                demand: true,
                backing: crate::process::MmapBacking::Anonymous,
            });

            // Apply Binary Permission Model tier result.
            // Tier 1/4: replace both sets.
            // Tier 2/3 (None): leave sets untouched — permissiond will set them.
            if let Some((new_perms, new_actions)) = exec_permission_tier_result.clone() {
                process.granted_permissions = new_perms;
                process.granted_actions = new_actions;
            }

            // Patch the exception frame in-place to redirect to the new entry.
            (*frame).elr = loaded.entry_point;
            (*frame).spsr = 0; // EL0 state
            (*frame).sp = loaded.initial_stack_pointer;
            crate::uart::puts("[exec] sp=");
            crate::uart::put_hex(loaded.initial_stack_pointer);
            crate::uart::puts(" demand=[");
            crate::uart::put_hex(loaded.stack_demand_base);
            crate::uart::puts(", ");
            crate::uart::put_hex(loaded.stack_demand_top);
            crate::uart::puts(")\r\n");
            // Clear GP registers x0–x30, then set AArch64 SysV ABI entry args:
            //   x0 = argc, x1 = argv VA, x2 = envp VA.
            // x0 is written by dispatch() using this function's return value.
            // x1 and x2 are set here because dispatch() only forward-writes x0.
            // Reference: AArch64 SYSV ABI §3.4.1 — initial stack and register state.
            for reg in (*frame).x.iter_mut() {
                *reg = 0;
            }
            (*frame).x[1] = loaded.argv_va;
            (*frame).x[2] = loaded.envp_va;

            // Store the new environment in the process for getenv/putenv/execve.
            process.environ.clear();
            for env_entry in envp {
                if let Ok(s) = core::str::from_utf8(env_entry) {
                    process.environ.push(alloc::string::String::from(s));
                }
            }

            // Activate the new page table.
            if let Some(page_table) = &process.page_table {
                page_table.activate_el0();
            }
        }
    });

    // exec() does not return to userspace on success — ELR/SP were redirected.
    // dispatch() writes this return value into x0, which _start receives as argc.
    // Reference: AArch64 SYSV ABI — _start(x0=argc, x1=argv, x2=envp).
    loaded.argc as i64
}

// ---------------------------------------------------------------------------
// sys_getpid
// ---------------------------------------------------------------------------

unsafe fn sys_getpid() -> i64 {
    // POSIX: getpid() returns the thread group ID (tgid), not the per-thread PID.
    // All threads in the same group return the same value.
    // Reference: Linux kernel/sys.c sys_getpid() — returns task->tgid.
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|process| process.tgid as i64)
            .unwrap_or(0)
    })
}

// ---------------------------------------------------------------------------
// sys_clock_gettime — return nanosecond timestamp from CNTPCT_EL0
// ---------------------------------------------------------------------------

/// POSIX CLOCK_REALTIME: wall-clock time (seconds since Unix epoch).
/// Value 0 matches both POSIX and Linux ABI.
const CLOCK_REALTIME: i32 = 0;

/// POSIX CLOCK_MONOTONIC: time since an unspecified point (here: boot).
/// Value 1 matches both POSIX and Linux ABI.
const CLOCK_MONOTONIC: i32 = 1;

unsafe fn sys_clock_gettime(clock_id: i32, timespec_ptr: *mut u64) -> i64 {
    if clock_id != CLOCK_REALTIME && clock_id != CLOCK_MONOTONIC {
        return EINVAL;
    }
    if timespec_ptr.is_null() || (timespec_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }

    // struct timespec { time_t tv_sec; long tv_nsec; } — two 8-byte words.
    match clock_id {
        CLOCK_MONOTONIC => {
            // Read hardware counter and frequency for nanosecond-precision monotonic time.
            let cntpct: u64;
            let cntfrq: u64;
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) cntpct, options(nostack, nomem));
            core::arch::asm!("mrs {}, cntfrq_el0", out(reg) cntfrq, options(nostack, nomem));
            let seconds     = cntpct / cntfrq;
            let nanoseconds = (cntpct % cntfrq) * 1_000_000_000 / cntfrq;
            *timespec_ptr        = seconds;
            *timespec_ptr.add(1) = nanoseconds;
        }
        CLOCK_REALTIME => {
            // Use PL031-derived wall clock: boot-time epoch + elapsed ticks.
            let tick = crate::platform::qemu_virt::timer::current_tick();
            let tick_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
            let (seconds, nanoseconds) =
                crate::platform::qemu_virt::rtc::realtime_now(tick, tick_ms);
            *timespec_ptr        = seconds;
            *timespec_ptr.add(1) = nanoseconds;
        }
        _ => return EINVAL,
    }
    0
}

// ---------------------------------------------------------------------------
// sys_nanosleep — put the calling process to sleep for the requested duration
// ---------------------------------------------------------------------------

/// `sys_nanosleep(req, rem)` — sleep for the duration specified by `*req`.
///
/// If a signal interrupts the sleep, writes the remaining time into `*rem`
/// (if `rem` is a valid user pointer) and returns `EINTR`.
///
/// Reference: POSIX.1-2017 nanosleep(2).
unsafe fn sys_nanosleep(timespec_ptr: *const u64, rmtp: *mut u64) -> i64 {
    if timespec_ptr.is_null() || (timespec_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }

    let seconds = *timespec_ptr;
    let nanoseconds = *timespec_ptr.add(1);

    // Convert the requested duration to kernel ticks.
    // One tick = TICK_INTERVAL_MS milliseconds = TICK_INTERVAL_MS * 1_000_000 ns.
    // We add one extra tick to guarantee at least the requested duration elapses
    // (the current tick may be nearly expired when we read it).
    let tick_interval_ns: u64 =
        crate::platform::qemu_virt::timer::TICK_INTERVAL_MS * 1_000_000;
    let total_ns = seconds
        .saturating_mul(1_000_000_000)
        .saturating_add(nanoseconds);
    let ticks_to_sleep = total_ns / tick_interval_ns + 1;

    if ticks_to_sleep == 0 {
        // Zero-duration sleep — just yield.
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.schedule();
        });
        return 0;
    }

    let now_tick = crate::platform::qemu_virt::timer::current_tick();
    let wake_at_tick = now_tick.saturating_add(ticks_to_sleep);

    // Transition the current process to Sleeping, then yield.
    // The scheduler's wake-up logic (in schedule()) will move it back to
    // Ready once current_tick() >= wake_at_tick.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.state = crate::process::ProcessState::Sleeping { wake_at_tick };
        }
        // schedule() will see state != Running and will not re-enqueue us.
        scheduler.schedule();
    });

    // After waking, check whether a signal caused the early wake-up.
    // `pending_signals != 0` means at least one signal is queued; deliver_pending_signals
    // (called by dispatch() after this function returns) will handle it.
    // POSIX.1-2017 nanosleep(2): if interrupted by a signal, write remaining
    // time to *rmtp and return EINTR.
    let has_pending_signal = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.pending_signals.load(core::sync::atomic::Ordering::Acquire) != 0)
            .unwrap_or(false)
    });

    if has_pending_signal {
        let woke_at = crate::platform::qemu_virt::timer::current_tick();
        // Write remaining time into *rmtp if the pointer is valid.
        if !rmtp.is_null() && (rmtp as u64) < crate::process::USER_ADDR_LIMIT
            && (rmtp as u64).saturating_add(16) <= crate::process::USER_ADDR_LIMIT
        {
            let remaining_ticks = wake_at_tick.saturating_sub(woke_at);
            let remaining_ns = remaining_ticks.saturating_mul(tick_interval_ns);
            let remaining_secs = remaining_ns / 1_000_000_000;
            let remaining_nsec = remaining_ns % 1_000_000_000;
            *rmtp = remaining_secs;
            *rmtp.add(1) = remaining_nsec;
        }
        return EINTR;
    }

    0
}

// ---------------------------------------------------------------------------
// sys_kill — send a signal to a process
// ---------------------------------------------------------------------------

unsafe fn sys_kill(target_pid: i32, signal_number: i32) -> i64 {
    if target_pid <= 0 {
        // POSIX.1-2017 kill(2): pid == 0 sends to the process group (not
        // supported yet); pid < 0 sends to a group (not supported yet).
        // ESRCH would mean "process not found" — incorrect here.
        // EPERM is the closest appropriate error for "not permitted / not
        // implemented" at this scope level.
        return EPERM;
    }
    if signal_number < 0 || signal_number as usize >= crate::process::SIGNAL_COUNT {
        return EINVAL;
    }
    let pid = crate::process::Pid::new(target_pid as u16, 1);
    crate::scheduler::with_scheduler(|scheduler| {
        if scheduler.send_signal_to(pid, signal_number as u8) {
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigaction — register a signal handler
// ---------------------------------------------------------------------------

unsafe fn sys_sigaction(
    signal_number: i32,
    handler_va: u64,
    old_handler_ptr: *mut u64,
    sa_flags: u32,
) -> i64 {
    if signal_number <= 0 || signal_number as usize >= crate::process::SIGNAL_COUNT {
        return EINVAL;
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            // Return the old handler if requested.
            if !old_handler_ptr.is_null() && (old_handler_ptr as u64) < crate::process::USER_ADDR_LIMIT {
                let old = match process.signal_handlers[signal_number as usize] {
                    crate::process::SignalAction::Handler { va, .. } => va,
                    crate::process::SignalAction::Ignore => u64::MAX - 1,
                    crate::process::SignalAction::Default => 0,
                };
                *old_handler_ptr = old;
            }

            let on_stack = (sa_flags & crate::process::SA_ONSTACK) != 0;
            let action = match handler_va {
                0 => crate::process::SignalAction::Default,
                u64::MAX => crate::process::SignalAction::Ignore,
                va => crate::process::SignalAction::Handler { va, on_stack },
            };

            match process.set_signal_handler(signal_number as u8, action) {
                Ok(()) => 0,
                Err(_) => EINVAL,
            }
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getppid
// ---------------------------------------------------------------------------

unsafe fn sys_getppid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.current_process() {
            Some(process) => match process.parent_pid {
                Some(ppid) => ppid.index as i64,
                None => 0,
            },
            None => ESRCH,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_creat — create or truncate a file via VFS
// ---------------------------------------------------------------------------

/// flags: 0 = truncate (O_TRUNC, default), 1 = no truncate (O_APPEND).
unsafe fn sys_creat(name_ptr: *const u8, name_length: usize, flags: u32) -> i64 {
    if name_ptr.is_null() || (name_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name_raw = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let truncate = (flags & 1) == 0; // flag bit 0 clear → O_TRUNC

    // Resolve relative paths against cwd before VFS lookup.
    let name_abs = resolve_to_absolute(name_raw);
    let name = name_abs.as_str();

    // Resolve or create via VFS.
    let inode = match crate::fs::vfs_resolve(name, None) {
        Ok(existing) => {
            // File exists: truncate if requested.
            if truncate { let _ = existing.truncate(0); }
            existing
        }
        Err(_) => {
            // File does not exist: create it.
            let (parent, file_name) = match crate::fs::vfs_resolve_parent(name) {
                Ok(pair) => pair,
                Err(error) => return error.to_errno(),
            };
            match parent.create(&file_name) {
                Ok(new_inode) => new_inode,
                Err(error) => return error.to_errno(),
            }
        }
    };

    let descriptor = crate::fs::vfs::FileDescriptor::InoFile { inode, position: 0 };
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc { Some(arc) => arc, None => return ESRCH };
    let mut guard = fd_table_arc.lock();
    let fd = guard.install(descriptor);
    if fd < 0 { EMFILE as i64 } else { fd as i64 }
}

// ---------------------------------------------------------------------------
// sys_unlink — delete a file
// ---------------------------------------------------------------------------

unsafe fn sys_unlink(name_ptr: *const u8, name_length: usize) -> i64 {
    if name_ptr.is_null() || (name_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name_raw = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let name_abs = resolve_to_absolute(name_raw);
    let name = name_abs.as_str();

    // Resolve to parent + name, then call unlink via VFS.
    match crate::fs::vfs_resolve_parent(name) {
        Ok((parent, file_name)) => {
            match parent.unlink(&file_name) {
                Ok(()) => 0,
                Err(error) => error.to_errno(),
            }
        }
        Err(error) => error.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_fstat — get file metadata
// ---------------------------------------------------------------------------

unsafe fn sys_fstat(name_ptr: *const u8, name_length: usize, stat_ptr: *mut u64) -> i64 {
    if name_ptr.is_null() || (name_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let name_bytes = core::slice::from_raw_parts(name_ptr, name_length);
    let name = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Resolve via VFS (covers tmpfs, FAT32, devfs, procfs, ramfs via exec path).
    let (size, file_type) = match crate::fs::vfs_resolve(name, None) {
        Ok(inode) => {
            let stat = inode.stat();
            let file_type_code = match inode.inode_type() {
                crate::fs::InodeType::RegularFile => 1i32,
                crate::fs::InodeType::Directory   => 2i32,
                crate::fs::InodeType::CharDevice  => 3i32,
                crate::fs::InodeType::Fifo        => 4i32,
                crate::fs::InodeType::Symlink     => 5i32,
            };
            (stat.size, file_type_code)
        }
        Err(_) => {
            // Fall back to ramfs for files not in VFS.
            match crate::fs::ramfs_find(name) {
                Some(data) => (data.len() as u64, 1i32),
                None => return ENOENT,
            }
        }
    };

    if !stat_ptr.is_null() && (stat_ptr as u64) < crate::process::USER_ADDR_LIMIT {
        *stat_ptr = size;
        *stat_ptr.add(1) = file_type as u64;
    }
    0
}

// ---------------------------------------------------------------------------
// sys_setfgpid — set foreground process
// ---------------------------------------------------------------------------

unsafe fn sys_setfgpid(pid_arg: i32) -> i64 {
    if pid_arg <= 0 {
        return EINVAL;
    }
    let pid = crate::process::Pid::new(pid_arg as u16, 1);
    crate::scheduler::with_scheduler(|scheduler| {
        // Clear foreground from all processes, set on target.
        for slot_index in 0..crate::scheduler::PID_MAX {
            if let Some(process) = scheduler.process_mut(crate::process::Pid::new(slot_index as u16, 1)) {
                process.is_foreground = false;
            }
        }
        if let Some(process) = scheduler.process_mut(pid) {
            process.is_foreground = true;
            0i64
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_disk_info — return disk capacity and FAT32 info
// ---------------------------------------------------------------------------

unsafe fn sys_disk_info(buf_ptr: *mut u64) -> i64 {
    if buf_ptr.is_null() || (buf_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let capacity = crate::hal::disk::capacity();
    *buf_ptr = capacity;
    0
}

// ---------------------------------------------------------------------------
// sys_getrandom — fill a user buffer with pseudo-random bytes
// ---------------------------------------------------------------------------

/// Fill `buf[0..len]` with bytes from the kernel entropy pool.
///
/// The entropy pool is seeded from CNTPCT_EL0 + CNTFRQ_EL0 + TTBR1_EL1.
/// This is NOT cryptographically secure (no hardware TRNG on cortex-a72),
/// but it is suitable for ASLR, salts, and non-security-critical randomness.
///
/// Flags argument is ignored (Linux getrandom flags: GRND_NONBLOCK, GRND_RANDOM).
///
/// Reference: Linux sys_getrandom (random.c), POSIX.1-2017 §getentropy.
unsafe fn sys_getrandom(buf_ptr: *mut u8, len: usize) -> i64 {
    if buf_ptr.is_null()
        || len == 0
        || (buf_ptr as u64) >= crate::process::USER_ADDR_LIMIT
        || (buf_ptr as u64).saturating_add(len as u64) > crate::process::USER_ADDR_LIMIT
    {
        return EINVAL;
    }

    let mut written = 0usize;

    while written < len {
        // Generate 8 bytes of entropy per iteration using the same mix as ASLR.
        let cntpct: u64;
        let cntfrq: u64;
        let ttbr1: u64;
        core::arch::asm!(
            "mrs {cntpct}, cntpct_el0",
            "mrs {cntfrq}, cntfrq_el0",
            "mrs {ttbr1}, ttbr1_el1",
            cntpct = out(reg) cntpct,
            cntfrq = out(reg) cntfrq,
            ttbr1  = out(reg) ttbr1,
            options(nostack, nomem)
        );

        static ENTROPY_COUNTER: core::sync::atomic::AtomicU64 =
            core::sync::atomic::AtomicU64::new(0xDEAD_BEEF_0000_0001);
        let counter = ENTROPY_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let mut state = cntpct
            ^ cntfrq.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (ttbr1 >> 12)
            ^ counter.wrapping_mul(0x6c62_272e_07bb_0142);

        // Xorshift64 mixing.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;

        // Copy up to 8 bytes from `state` into `buf`.
        let remaining = len - written;
        let chunk = if remaining >= 8 { 8 } else { remaining };
        let state_bytes = state.to_le_bytes();
        core::ptr::copy_nonoverlapping(state_bytes.as_ptr(), buf_ptr.add(written), chunk);
        written += chunk;
    }

    len as i64
}

// ---------------------------------------------------------------------------
// sys_sigreturn — restore context after signal handler
// ---------------------------------------------------------------------------

unsafe fn sys_sigreturn(frame: *mut ExceptionFrame) -> i64 {
    // Restore pre-signal CPU state from the signal frame written by
    // deliver_pending_signals().
    //
    // Signal frame layout at sp (set up by deliver_pending_signals):
    //   [sp + 0]  saved ELR_EL0  (pre-signal PC)
    //   [sp + 8]  saved SPSR_EL1 (pre-signal pstate)
    //
    // After restoring, advance sp by 32 to discard the frame.
    //
    // Reference: POSIX.1-2017 sigreturn(2) — restore pre-signal context.
    let sp = (*frame).sp;
    (*frame).elr  = core::ptr::read(sp as *const u64);
    (*frame).spsr = core::ptr::read((sp + 8) as *const u64);
    (*frame).sp   = sp.wrapping_add(32);

    // If the process was executing on the alternate signal stack, mark that
    // we have returned from it so the next signal can use it again.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.on_signal_stack = false;
        }
    });

    // Return value of sigreturn is not observed — the restored ELR redirects
    // execution back to the pre-signal instruction.
    0
}

// ---------------------------------------------------------------------------
// sys_sigaltstack — set / query per-process alternate signal stack
// ---------------------------------------------------------------------------

/// Kernel-visible layout of the `stack_t` / `sigaltstack` struct.
///
/// Matches the layout expected by POSIX `sigaltstack(2)`:
///   Offset 0: ss_sp   (u64) — base pointer of the alternate stack region
///   Offset 8: ss_flags (u32) — SS_DISABLE (4) = disabled, SS_ONSTACK (1) = in use
///   Offset 16: ss_size (u64) — size of the region in bytes
///
/// Reference: POSIX.1-2017 `sys/signal.h`, Linux `asm/signal.h`.
#[repr(C)]
struct UserSignalStack {
    ss_sp:    u64,
    ss_flags: u32,
    _pad:     u32,
    ss_size:  u64,
}

/// `sigaltstack(new_stack_ptr, old_stack_ptr) → 0 | -errno`
///
/// If `new_stack_ptr` is non-null: install the described alternate stack.
/// If `old_stack_ptr` is non-null: write the current alternate stack state.
/// Either pointer may be null (query-only or set-only).
///
/// Returns `EPERM` if called while executing on the alternate stack (`SS_ONSTACK`).
/// Returns `EINVAL` if the new stack size is smaller than the POSIX minimum (2048 bytes).
///
/// Reference: POSIX.1-2017 `sigaltstack(2)`.
unsafe fn sys_sigaltstack(new_stack_ptr: u64, old_stack_ptr: u64) -> i64 {
    use crate::process::{SignalStack, SS_DISABLE, SS_ONSTACK};

    // Reject pointers outside user address space.
    let new_ptr_valid = new_stack_ptr != 0
        && new_stack_ptr < crate::process::USER_ADDR_LIMIT;
    let old_ptr_valid = old_stack_ptr != 0
        && old_stack_ptr < crate::process::USER_ADDR_LIMIT;

    crate::scheduler::with_scheduler(|scheduler| {
        let process = match scheduler.current_process_mut() {
            Some(p) => p,
            None => return ESRCH,
        };

        // Cannot change the alternate stack while executing on it.
        if new_ptr_valid && process.on_signal_stack {
            return -(crate::process::SS_ONSTACK as i64); // EPERM-like; conventionally EINVAL on Linux
        }

        // Write the old stack state if requested.
        if old_ptr_valid {
            let out = &mut *(old_stack_ptr as *mut UserSignalStack);
            match process.signal_stack {
                Some(ss) => {
                    out.ss_sp    = ss.base;
                    out.ss_flags = if process.on_signal_stack { SS_ONSTACK } else { 0 };
                    out.ss_size  = ss.size as u64;
                }
                None => {
                    out.ss_sp    = 0;
                    out.ss_flags = SS_DISABLE;
                    out.ss_size  = 0;
                }
            }
        }

        // Install the new stack if requested.
        if new_ptr_valid {
            let input = &*(new_stack_ptr as *const UserSignalStack);
            if input.ss_flags & SS_DISABLE != 0 {
                process.signal_stack = None;
            } else {
                // POSIX minimum alternate stack size: MINSIGSTKSZ = 2048 bytes.
                // Reference: POSIX.1-2017 `<signal.h>`.
                const MINSIGSTKSZ: usize = 2048;
                if (input.ss_size as usize) < MINSIGSTKSZ {
                    return EINVAL;
                }
                process.signal_stack = Some(SignalStack {
                    base:  input.ss_sp,
                    size:  input.ss_size as usize,
                    flags: 0,
                });
            }
        }

        0
    })
}

// ---------------------------------------------------------------------------
// Phase 7 — Scheduler: nice, rlimits, process groups, sessions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_nice — add increment to the caller's nice value
// ---------------------------------------------------------------------------

unsafe fn sys_nice(increment: i32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            let new_nice = (process.nice as i32).saturating_add(increment)
                .max(crate::process::NICE_MIN as i32)
                .min(crate::process::NICE_MAX as i32) as i8;
            process.nice = new_nice;
            new_nice as i64
        } else {
            EINVAL
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getpriority / sys_setpriority — get/set nice value directly
// ---------------------------------------------------------------------------

unsafe fn sys_getpriority() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.nice as i64)
            .unwrap_or(EINVAL)
    })
}

unsafe fn sys_setpriority(nice: i32) -> i64 {
    if nice < crate::process::NICE_MIN as i32 || nice > crate::process::NICE_MAX as i32 {
        return EINVAL;
    }
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.nice = nice as i8;
            0
        } else {
            EINVAL
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getrlimit / sys_setrlimit — query/change resource limits
//
// ABI: getrlimit(resource, rlim_ptr) where rlim_ptr points to two u64 words:
//   [0] = rlim_cur (soft limit)
//   [1] = rlim_max (hard limit) — we store soft == hard for simplicity
// ---------------------------------------------------------------------------

unsafe fn sys_getrlimit(resource: u32, rlim_ptr: *mut u64) -> i64 {
    if rlim_ptr.is_null() || (rlim_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }

    let limit_value = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process().map(|p| {
            match resource {
                crate::process::RLIMIT_NOFILE => p.resource_limits.open_files,
                crate::process::RLIMIT_AS     => p.resource_limits.address_space_bytes,
                crate::process::RLIMIT_STACK  => p.resource_limits.stack_bytes,
                _                             => u64::MAX,
            }
        })
    });

    match limit_value {
        Some(val) => {
            *rlim_ptr         = val; // rlim_cur
            *rlim_ptr.add(1)  = val; // rlim_max (same as soft)
            0
        }
        None => EINVAL,
    }
}

unsafe fn sys_setrlimit(resource: u32, new_limit: u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            match resource {
                crate::process::RLIMIT_NOFILE => {
                    process.resource_limits.open_files = new_limit;
                    0
                }
                crate::process::RLIMIT_AS => {
                    process.resource_limits.address_space_bytes = new_limit;
                    0
                }
                crate::process::RLIMIT_STACK => {
                    process.resource_limits.stack_bytes = new_limit;
                    0
                }
                _ => EINVAL,
            }
        } else {
            EINVAL
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getpgrp / sys_setpgid — process group management
// ---------------------------------------------------------------------------

unsafe fn sys_getpgrp() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.pgid as i64)
            .unwrap_or(ESRCH)
    })
}

/// `setpgid(pid, pgid)` — set the process group of `pid` to `pgid`.
///
/// If `pid` is 0, the caller's own PID is used.
/// If `pgid` is 0, the target's PID is used (makes it a group leader).
///
/// Reference: POSIX.1-2017 `setpgid(2)`.
unsafe fn sys_setpgid(pid: i32, pgid: i32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        let target_pid = if pid == 0 {
            scheduler.current_pid()
        } else {
            crate::process::Pid::new(pid as u16, 1)
        };

        let new_pgid = if pgid == 0 {
            target_pid.index as u32
        } else {
            pgid as u32
        };

        if let Some(process) = scheduler.process_mut(target_pid) {
            process.pgid = new_pgid;
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getsid / sys_setsid — session management
// ---------------------------------------------------------------------------

unsafe fn sys_getsid(pid: i32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        let target_pid = if pid == 0 {
            scheduler.current_pid()
        } else {
            crate::process::Pid::new(pid as u16, 1)
        };
        scheduler.process(target_pid)
            .map(|p| p.sid as i64)
            .unwrap_or(ESRCH)
    })
}

/// `setsid()` — create a new session with the calling process as session leader.
///
/// The process becomes the leader of a new session and a new process group.
/// Returns the new session ID (= caller's PID) on success.
///
/// Fails with EPERM if the caller is already a process group leader.
///
/// Reference: POSIX.1-2017 `setsid(2)`.
unsafe fn sys_setsid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        let current_pid = scheduler.current_pid();
        let pid_u32 = current_pid.index as u32;

        if let Some(process) = scheduler.current_process_mut() {
            // Cannot call setsid() if already a process group leader.
            if process.pgid == pid_u32 {
                return EPERM;
            }
            process.sid  = pid_u32;
            process.pgid = pid_u32;
            pid_u32 as i64
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_tcgetpgrp / sys_tcsetpgrp — terminal foreground process group
//
// Simplified: we maintain a single global foreground PGID per TTY.
// The `fd` argument is accepted but ignored (single TTY).
// ---------------------------------------------------------------------------

/// Global foreground process group ID for the terminal.
///
/// Set by `tcsetpgrp()`; read by `tcgetpgrp()` and used by the TTY driver
/// to route SIGINT/SIGTSTP to the foreground group.
///
/// Initial value 0 = no foreground group set.
static TERMINAL_FOREGROUND_PGID: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Return the foreground process group ID for the terminal.
pub fn terminal_foreground_pgid() -> u32 {
    TERMINAL_FOREGROUND_PGID.load(core::sync::atomic::Ordering::Relaxed)
}

unsafe fn sys_tcgetpgrp(_fd: i32) -> i64 {
    TERMINAL_FOREGROUND_PGID.load(core::sync::atomic::Ordering::Relaxed) as i64
}

unsafe fn sys_tcsetpgrp(_fd: i32, pgid: i32) -> i64 {
    if pgid <= 0 {
        return EINVAL;
    }
    TERMINAL_FOREGROUND_PGID.store(pgid as u32, core::sync::atomic::Ordering::Relaxed);
    0
}

// ---------------------------------------------------------------------------
// Phase 8 — POSIX syscalls: uname, sysinfo, sigprocmask, sigpending,
//           sigsuspend, getrusage, prctl, gettimeofday, poll
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_uname — return OS name, version, and architecture
//
// ABI: uname(buf) where buf points to 6 fields of 65 bytes each (POSIX utsname).
//   [0..65]   sysname
//   [65..130] nodename
//   [130..195] release
//   [195..260] version
//   [260..325] machine
//   [325..390] domainname
// ---------------------------------------------------------------------------

unsafe fn sys_uname(buf: *mut u8) -> i64 {
    if !validate_user_pointer(buf as u64, 390) {
        return EINVAL;
    }

    const FIELD_LEN: usize = 65;
    let fields: [&[u8]; 6] = [
        b"Bazzulto",
        b"bazzulto",
        b"0.1.0",
        b"Bazzulto 0.1.0 (AArch64)",
        b"aarch64",
        b"",
    ];

    for (i, field) in fields.iter().enumerate() {
        let dst = buf.add(i * FIELD_LEN);
        let copy_len = field.len().min(FIELD_LEN - 1);
        core::ptr::copy_nonoverlapping(field.as_ptr(), dst, copy_len);
        *dst.add(copy_len) = 0; // NUL terminate
    }

    0
}

// ---------------------------------------------------------------------------
// sys_sysinfo — return system statistics
//
// ABI: sysinfo(info_ptr) where info_ptr points to a struct of u64 fields:
//   [0]  uptime in seconds
//   [1]  total RAM in bytes
//   [2]  free RAM in bytes
//   [3]  number of processes
// (Simplified subset of Linux struct sysinfo)
// ---------------------------------------------------------------------------

unsafe fn sys_sysinfo(info_ptr: *mut u64) -> i64 {
    if !validate_user_pointer(info_ptr as u64, 4 * 8) {
        return EINVAL;
    }

    let uptime_seconds = crate::platform::qemu_virt::timer::current_tick()
        * crate::platform::qemu_virt::timer::TICK_INTERVAL_MS
        / 1000;

    let (total_ram, free_ram) = crate::memory::physical_stats();

    let process_count = crate::scheduler::with_scheduler(|s| s.alive_process_count());

    *info_ptr          = uptime_seconds;
    *info_ptr.add(1)   = total_ram;
    *info_ptr.add(2)   = free_ram;
    *info_ptr.add(3)   = process_count as u64;

    0
}

// ---------------------------------------------------------------------------
// sys_sigprocmask — examine and change blocked signals
//
// how:  0 = SIG_BLOCK   (mask |= set)
//       1 = SIG_UNBLOCK (mask &= ~set)
//       2 = SIG_SETMASK (mask  = set)
// ---------------------------------------------------------------------------

const SIG_BLOCK:   i32 = 0;
const SIG_UNBLOCK: i32 = 1;
const SIG_SETMASK: i32 = 2;

/// Bits for signals that can never be blocked (SIGKILL=9, SIGSTOP=19).
const UNBLOCKABLE_SIGNALS_MASK: u64 = (1u64 << 9) | (1u64 << 19);

unsafe fn sys_sigprocmask(how: i32, set_ptr: *const u64, old_set_ptr: *mut u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            // Return old mask if requested.
            if !old_set_ptr.is_null()
                && (old_set_ptr as u64) < crate::process::USER_ADDR_LIMIT
            {
                *old_set_ptr = process.signal_mask;
            }

            // Apply new mask if provided.
            if !set_ptr.is_null()
                && (set_ptr as u64) < crate::process::USER_ADDR_LIMIT
            {
                let new_set = *set_ptr & !UNBLOCKABLE_SIGNALS_MASK;
                process.signal_mask = match how {
                    SIG_BLOCK   => process.signal_mask | new_set,
                    SIG_UNBLOCK => process.signal_mask & !new_set,
                    SIG_SETMASK => new_set,
                    _           => return EINVAL,
                };
            }
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigpending — return set of pending signals
// ---------------------------------------------------------------------------

unsafe fn sys_sigpending(set_ptr: *mut u64) -> i64 {
    if set_ptr.is_null() || (set_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process() {
            *set_ptr = process.pending_signals.load(core::sync::atomic::Ordering::Acquire)
                & process.signal_mask;
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigsuspend — replace signal mask and suspend until signal arrives
// ---------------------------------------------------------------------------

unsafe fn sys_sigsuspend(frame: *mut ExceptionFrame, mask: u64) -> i64 {
    // Install the new mask (clearing unblockable bits).
    let applied_mask = mask & !UNBLOCKABLE_SIGNALS_MASK;
    let old_mask = crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            let old = process.signal_mask;
            process.signal_mask = applied_mask;
            old
        } else {
            0
        }
    });

    // Block until a non-masked signal is pending.
    loop {
        // Check for deliverable signal.
        let has_signal = crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process() {
                let pending = process.pending_signals.load(core::sync::atomic::Ordering::Acquire);
                (pending & !process.signal_mask) != 0
            } else {
                true // exit loop if process gone
            }
        });

        if has_signal {
            break;
        }

        // No deliverable signal — yield.
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.schedule();
        });
    }

    // Restore old mask.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.signal_mask = old_mask;
        }
    });

    // Deliver any pending signals before returning.
    deliver_pending_signals(frame);

    // sigsuspend always returns -EINTR.
    EINTR
}

// ---------------------------------------------------------------------------
// sys_getrusage — return resource usage statistics
//
// who: 0 = RUSAGE_SELF, -1 = RUSAGE_CHILDREN (stub: same as self)
//
// ABI: getrusage(who, usage_ptr) where usage_ptr points to two timeval fields:
//   [0..1] ru_utime (user time): seconds + microseconds
//   [2..3] ru_stime (system time): seconds + microseconds (stub: 0)
// ---------------------------------------------------------------------------

unsafe fn sys_getrusage(who: i32, usage_ptr: *mut u64) -> i64 {
    let _ = who;
    if !validate_user_pointer(usage_ptr as u64, 4 * 8) {
        return EINVAL;
    }

    let (user_ticks, sys_ticks) = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| (p.user_ticks, p.sys_time_ticks))
            .unwrap_or((0, 0))
    });

    // Convert ticks to (seconds, microseconds).
    // TICK_INTERVAL_MS is the timer period in ms; ticks_per_second = 1000 / interval.
    let ticks_per_second = 1000 / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
    let user_seconds  = user_ticks / ticks_per_second;
    let user_useconds = (user_ticks % ticks_per_second)
        * (1_000_000 / ticks_per_second);
    // sys_time_ticks counts one tick per syscall entry.  Reuse the same
    // ticks_per_second denominator so both times are on the same scale.
    let sys_seconds   = sys_ticks / ticks_per_second;
    let sys_useconds  = (sys_ticks % ticks_per_second)
        * (1_000_000 / ticks_per_second);

    *usage_ptr          = user_seconds;  // ru_utime.tv_sec
    *usage_ptr.add(1)   = user_useconds; // ru_utime.tv_usec
    *usage_ptr.add(2)   = sys_seconds;   // ru_stime.tv_sec
    *usage_ptr.add(3)   = sys_useconds;  // ru_stime.tv_usec

    0
}

// ---------------------------------------------------------------------------
// sys_prctl — process control operations
//
// Only PR_SET_NAME (15) and PR_GET_NAME (16) are implemented.
// ---------------------------------------------------------------------------

const PR_SET_NAME: i32 = 15;
const PR_GET_NAME: i32 = 16;

unsafe fn sys_prctl(option: i32, name_ptr: *const u8, name_len: usize) -> i64 {
    match option {
        PR_SET_NAME => {
            if !validate_user_pointer(name_ptr as u64, 1) {
                return EINVAL;
            }
            let copy_len = name_len.min(15);
            let mut name_buf = [0u8; 16];
            core::ptr::copy_nonoverlapping(name_ptr, name_buf.as_mut_ptr(), copy_len);
            name_buf[copy_len] = 0;

            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process_mut() {
                    process.name = name_buf;
                    0
                } else {
                    ESRCH
                }
            })
        }
        PR_GET_NAME => {
            if !validate_user_pointer(name_ptr as u64, 16) {
                return EINVAL;
            }
            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process() {
                    core::ptr::copy_nonoverlapping(
                        process.name.as_ptr(),
                        name_ptr as *mut u8,
                        16,
                    );
                    0
                } else {
                    ESRCH
                }
            })
        }
        _ => EINVAL,
    }
}

// ---------------------------------------------------------------------------
// sys_gettimeofday — return current time as (seconds, microseconds)
//
// ABI: gettimeofday(tv_ptr, tz_ptr) where tv_ptr points to two u64 words.
// Returns wall-clock time via the PL031 RTC snapshot plus elapsed ticks.
// ---------------------------------------------------------------------------

unsafe fn sys_gettimeofday(tv_ptr: *mut u64) -> i64 {
    if tv_ptr.is_null() || (tv_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return 0; // gettimeofday ignores null tv
    }

    let tick    = crate::platform::qemu_virt::timer::current_tick();
    let tick_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
    let (seconds, nanoseconds) =
        crate::platform::qemu_virt::rtc::realtime_now(tick, tick_ms);
    let useconds = nanoseconds / 1_000;

    *tv_ptr        = seconds;
    *tv_ptr.add(1) = useconds;
    0
}

// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
// Phase 9 — VFS syscalls
// ---------------------------------------------------------------------------

/// Helper: copy a user-supplied path into a kernel buffer.
///
/// Returns `None` if the pointer is invalid or the bytes are not valid UTF-8.
unsafe fn copy_user_path<'a>(
    name_ptr: *const u8,
    name_len: usize,
    buf: &'a mut [u8; 512],
) -> Option<&'a str> {
    if !validate_user_pointer(name_ptr as u64, name_len) || name_len > 511 {
        return None;
    }
    core::ptr::copy_nonoverlapping(name_ptr, buf.as_mut_ptr(), name_len);
    buf[name_len] = 0;
    core::str::from_utf8(&buf[..name_len]).ok()
}

// ---------------------------------------------------------------------------
// sys_chdir — change working directory
// ---------------------------------------------------------------------------

unsafe fn sys_chdir(path_ptr: *const u8, path_len: usize) -> i64 {
    let mut buf = [0u8; 512];
    let path = match copy_user_path(path_ptr, path_len, &mut buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Resolve path to inode.
    let cwd_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process().and_then(|p| p.cwd.clone())
    });
    let inode = match crate::fs::vfs_resolve(path, cwd_arc.as_ref()) {
        Ok(inode) => inode,
        Err(err) => return err.to_errno(),
    };

    if inode.inode_type() != crate::fs::InodeType::Directory {
        return -20; // ENOTDIR
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.cwd = Some(inode);
            // Update stored path string.
            if path.starts_with('/') {
                // Absolute path — normalize and store directly.
                process.cwd_path = normalize_cwd_path(path);
            } else {
                // Relative path — append to current cwd_path.
                let base = process.cwd_path.clone();
                process.cwd_path = normalize_cwd_path(&alloc::format!("{}/{}", base.trim_end_matches('/'), path));
            }
            0
        } else {
            ESRCH
        }
    })
}

/// Normalize an absolute path string: collapse `//`, `/./`, and `/../`.
/// Always returns an absolute path starting with `/`.
fn normalize_cwd_path(path: &str) -> alloc::string::String {
    let mut components: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => { components.pop(); }
            other => components.push(other),
        }
    }
    if components.is_empty() {
        return alloc::string::String::from("/");
    }
    let mut result = alloc::string::String::from("/");
    result.push_str(&components.join("/"));
    result
}

/// Resolve `path` to an absolute path.
///
/// - If `path` starts with `/` it is returned unchanged (already absolute).
/// - Otherwise the process's `cwd_path` is prepended, then `normalize_cwd_path`
///   is applied to collapse `.`, `..`, and double slashes.
///
/// The returned `String` is always absolute (starts with `/`).
fn resolve_to_absolute(path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        return normalize_cwd_path(path);
    }
    let cwd = unsafe { crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.cwd_path.clone())
            .unwrap_or_else(|| alloc::string::String::from("/"))
    }) };
    normalize_cwd_path(&alloc::format!("{}/{}", cwd.trim_end_matches('/'), path))
}

// ---------------------------------------------------------------------------
// sys_getcwd — return working directory path
//
// Simplified: writes "/" if cwd is not set, else writes "/cwd-placeholder".
// A full implementation requires walking the dentry tree upward.
// ---------------------------------------------------------------------------

unsafe fn sys_getcwd(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    if !validate_user_pointer(buf_ptr as u64, buf_len) || buf_len == 0 {
        return EINVAL;
    }

    let cwd_path = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.cwd_path.clone())
            .unwrap_or_else(|| alloc::string::String::from("/"))
    });

    let path_bytes = cwd_path.as_bytes();
    // Write path bytes + NUL terminator into the user buffer.
    let copy_len = path_bytes.len().min(buf_len - 1);
    core::ptr::copy_nonoverlapping(path_bytes.as_ptr(), buf_ptr, copy_len);
    buf_ptr.add(copy_len).write(0); // NUL terminator
    (copy_len + 1) as i64 // return length including NUL
}

// ---------------------------------------------------------------------------
// sys_umask — set and return the file creation mask
// ---------------------------------------------------------------------------
//
// umask(mask) → old_mask
//
// Sets the per-process file creation mask.  New files and directories have the
// bits in `mask` cleared from the mode argument passed to open() and mkdir().
// Returns the previous umask value.
//
// Only the lower 9 permission bits (0o777) of mask are significant; the
// upper bits (file type) are always ignored.
//
// Reference: POSIX.1-2017 §2.5.3.3 (File Creation Mask).

unsafe fn sys_umask(mask: u32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.current_process_mut() {
            Some(process) => {
                let old = process.umask;
                process.umask = mask & 0o777;
                old as i64
            }
            None => 0,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_mkdir — create directory
// ---------------------------------------------------------------------------

unsafe fn sys_mkdir(path_ptr: *const u8, path_len: usize, mode: u32) -> i64 {
    let mut buf = [0u8; 512];
    let path_raw = match copy_user_path(path_ptr, path_len, &mut buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let path_abs = resolve_to_absolute(path_raw);
    let path = path_abs.as_str();

    let (parent, dir_name) = match crate::fs::vfs_resolve_parent(path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    let umask = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process().map(|p| p.umask).unwrap_or(0o022)
    });
    // Directory type bits (0o040000) | permission bits with umask applied.
    let effective_mode = (0o040000u64)
        | (((mode as u64) & 0o777) & !((umask as u64) & 0o777));

    match parent.mkdir(&dir_name) {
        Ok(inode) => {
            let _ = inode.set_mode(effective_mode);
            0
        }
        Err(err) => err.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_rmdir — remove directory
// ---------------------------------------------------------------------------

unsafe fn sys_rmdir(path_ptr: *const u8, path_len: usize) -> i64 {
    let mut buf = [0u8; 512];
    let path = match copy_user_path(path_ptr, path_len, &mut buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let (parent, dir_name) = match crate::fs::vfs_resolve_parent(path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    // Verify the target is a directory before unlinking.
    let target = match parent.lookup(&dir_name) {
        Some(inode) => inode,
        None => return ENOENT,
    };
    if target.inode_type() != crate::fs::InodeType::Directory {
        return -20; // ENOTDIR
    }

    match parent.unlink(&dir_name) {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_rename — rename or move a file/directory
// ---------------------------------------------------------------------------

unsafe fn sys_rename(
    old_ptr: *const u8, old_len: usize,
    new_ptr: *const u8, new_len: usize,
) -> i64 {
    let mut old_buf = [0u8; 512];
    let mut new_buf = [0u8; 512];
    let old_path = match copy_user_path(old_ptr, old_len, &mut old_buf) {
        Some(p) => p,
        None => return EINVAL,
    };
    let new_path = match copy_user_path(new_ptr, new_len, &mut new_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Resolve source.
    let source_inode = match crate::fs::vfs_resolve(old_path, None) {
        Ok(inode) => inode,
        Err(err) => return err.to_errno(),
    };

    // Resolve source parent to unlink.
    let (old_parent, old_name) = match crate::fs::vfs_resolve_parent(old_path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    // Resolve destination parent to link.
    let (new_parent, new_name) = match crate::fs::vfs_resolve_parent(new_path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    // Link source inode at new location.
    if let Err(err) = new_parent.link_child(&new_name, source_inode) {
        return err.to_errno();
    }

    // Unlink from old location.
    match old_parent.unlink(&old_name) {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_getdents64 — read directory entries
//
// Kernel-internal dirent64 layout (matches Linux struct linux_dirent64):
//   u64  d_ino         — inode number
//   u64  d_off         — opaque offset (entry index)
//   u16  d_reclen      — size of this record
//   u8   d_type        — file type (DT_REG=8, DT_DIR=4, DT_CHR=2)
//   u8[] d_name        — NUL-terminated name
//
// Total header before name: 8+8+2+1 = 19 bytes; padded to 8-byte alignment.
// ---------------------------------------------------------------------------

unsafe fn sys_getdents64(fd: i32, buf_ptr: *mut u8, buf_len: usize) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(buf_ptr as u64, buf_len) {
        return EINVAL;
    }

    // Get the inode and current position from the InoFile descriptor.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let maybe = fd_table_arc.as_ref().and_then(|arc| {
        let guard = arc.lock();
        if let Some(crate::fs::vfs::FileDescriptor::InoFile { inode, position }) =
            guard.get(fd as usize)
        {
            Some((inode.clone(), *position as usize))
        } else {
            None
        }
    });

    let (inode, start_index) = match maybe {
        Some(pair) => pair,
        None => return EBADF,
    };

    if inode.inode_type() != crate::fs::InodeType::Directory {
        return -20; // ENOTDIR
    }

    let mut written: usize = 0;
    let mut index = start_index;

    loop {
        let entry = match inode.readdir(index) {
            Some(e) => e,
            None => break,
        };
        index += 1;

        let name_bytes = entry.name.as_bytes();
        // Header: d_ino(8) + d_off(8) + d_reclen(2) + d_type(1) = 19 bytes.
        const HEADER_SIZE: usize = 19;
        let record_size = (HEADER_SIZE + name_bytes.len() + 1 + 7) & !7;

        if written + record_size > buf_len {
            if written == 0 {
                return EINVAL; // buffer too small for even one entry
            }
            break;
        }

        let record_ptr = buf_ptr.add(written);
        (record_ptr as *mut u64).write_unaligned(entry.inode_number);
        (record_ptr.add(8) as *mut u64).write_unaligned(index as u64);
        (record_ptr.add(16) as *mut u16).write_unaligned(record_size as u16);
        let dtype: u8 = match entry.inode_type {
            crate::fs::InodeType::Directory   => 4,
            crate::fs::InodeType::RegularFile => 8,
            crate::fs::InodeType::CharDevice  => 2,
            // DT_FIFO = 1 (Linux d_type value for named pipes).
            crate::fs::InodeType::Fifo        => 1,
            // DT_LNK = 10 (Linux d_type value for symbolic links).
            crate::fs::InodeType::Symlink     => 10,
        };
        record_ptr.add(18).write(dtype);
        core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), record_ptr.add(19), name_bytes.len());
        record_ptr.add(19 + name_bytes.len()).write(0);

        written += record_size;
    }

    // Update position in the FD table.
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        if let Some(crate::fs::vfs::FileDescriptor::InoFile { position, .. }) =
            guard.get_mut(fd as usize)
        {
            *position = index as u64;
        }
    }

    written as i64
}

// ---------------------------------------------------------------------------
// sys_truncate_fd — truncate an open file to a given length
// ---------------------------------------------------------------------------

unsafe fn sys_truncate_fd(fd: i32, new_size: u64) -> i64 {
    if fd < 0 {
        return EBADF;
    }

    let inode = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            if let Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) =
                guard.get(fd as usize)
            {
                Some(inode.clone())
            } else {
                None
            }
        })
    };

    match inode {
        Some(inode) => match inode.truncate(new_size) {
            Ok(()) => 0,
            Err(err) => err.to_errno(),
        },
        None => EBADF,
    }
}

// ---------------------------------------------------------------------------
// sys_fsync — flush file data to storage
// ---------------------------------------------------------------------------

unsafe fn sys_fsync(fd: i32) -> i64 {
    if fd < 0 {
        return EBADF;
    }

    let inode = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            if let Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) =
                guard.get(fd as usize)
            {
                Some(inode.clone())
            } else {
                None
            }
        })
    };

    match inode {
        Some(inode) => match inode.fsync() {
            Ok(()) => 0,
            Err(err) => err.to_errno(),
        },
        None => EBADF,
    }
}

unsafe fn sys_poll(fds_ptr: *mut u64, nfds: usize, timeout_ms: i32) -> i64 {
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

    if !validate_user_pointer(fds_ptr as u64, nfds * 16) {
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
        // Each pollfd is laid out as two u64s:
        //   word0: fd (u32) | events (u32, upper)
        //   word1: revents (u32, lower — written by kernel)
        let word0 = *fds_ptr.add(index * 2);
        let fd = (word0 & 0xFFFF_FFFF) as i32;
        let events = ((word0 >> 32) & 0xFFFF) as u16;

        let revents: u16 = if fd < 0 {
            // Negative fd: skip
            0
        } else if guard.get(fd as usize).is_none() {
            POLLNVAL
        } else {
            // Simplified: report all requested events as ready.
            // A real implementation would check pipe buffer fill,
            // device readiness, etc.
            events & (POLLIN | POLLOUT)
        };

        // Write revents into word1 lower 32 bits.
        let word1_ptr = fds_ptr.add(index * 2 + 1);
        *word1_ptr = revents as u64;

        if revents != 0 && revents != POLLNVAL {
            ready_count += 1;
        }
    }
    ready_count
}

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
const TIOCGWINSZ: u32 = 0x5413;

/// TIOCSWINSZ — set window size.
/// Reference: Linux include/uapi/asm-generic/ioctls.h.
const TIOCSWINSZ: u32 = 0x5414;

/// TIOCGPTN — get PTY slave index number.
/// Reference: Linux include/uapi/linux/tty.h.
const TIOCGPTN: u32 = 0x80045430;

/// TIOCSPTLCK — set/clear PTY slave lock.
/// Reference: Linux include/uapi/linux/tty.h.
const TIOCSPTLCK: u32 = 0x40045431;

unsafe fn sys_ioctl(fd: i32, request: u32, arg: u64) -> i64 {
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
unsafe fn sys_tcgetattr(fd: i32, termios_ptr: *mut u8) -> i64 {
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
unsafe fn sys_tcsetattr(fd: i32, _optional_actions: i32, termios_ptr: *const u8) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(termios_ptr as u64, 48) {
        return EINVAL;
    }
    crate::drivers::tty::tty_tcsetattr(termios_ptr as *const crate::drivers::tty::Termios);
    0
}

// ---------------------------------------------------------------------------
// Phase 12 — epoll syscalls
// ---------------------------------------------------------------------------

/// `epoll_create1(flags)` — create a new epoll instance.
///
/// `flags`: currently only EPOLL_CLOEXEC (0x80000) is defined; we accept it
/// but do not enforce close-on-exec in the current implementation.
///
/// Returns the new file descriptor number, or a negative errno.
///
/// Reference: Linux `fs/eventpoll.c`.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn sys_epoll_create1(_flags: i32) -> i64 {
    let epoll_instance = crate::fs::epoll::EpollInstance::new();
    let inode_number = crate::fs::inode::Inode::stat(&*epoll_instance).inode_number;
    // Register in the global table so epoll_ctl / epoll_wait can find it.
    // SAFETY: single-core, IRQs disabled.
    EPOLL_INSTANCE_TABLE.register(inode_number, epoll_instance.clone());
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
        inode: epoll_instance,
        position: 0,
    };
    let new_fd = guard.install(descriptor);
    if new_fd < 0 { EMFILE as i64 } else { new_fd as i64 }
}

/// `epoll_ctl(epfd, op, fd, event_ptr)` — add/modify/remove interest.
///
/// `op` is one of EPOLL_CTL_ADD (1), EPOLL_CTL_DEL (2), EPOLL_CTL_MOD (3).
/// `event_ptr` points to a user-space `struct epoll_event` (events: u32, data: u64).
///
/// Returns 0 on success or a negative errno.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn sys_epoll_ctl(epfd: i32, operation: i32, watched_fd: i32, event_ptr: u64) -> i64 {
    use crate::fs::epoll::{EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};

    if epfd < 0 || watched_fd < 0 {
        return EBADF;
    }

    // DEL does not require a valid event_ptr.
    let (event_mask, user_data) = if operation == EPOLL_CTL_DEL {
        (0u32, 0u64)
    } else {
        if !validate_user_pointer(event_ptr, core::mem::size_of::<crate::fs::epoll::EpollEvent>()) {
            return EINVAL;
        }
        // SAFETY: event_ptr is validated above.
        let raw_event = core::ptr::read_unaligned(
            event_ptr as *const crate::fs::epoll::EpollEvent,
        );
        (raw_event.events, raw_event.data)
    };

    // Obtain the inode_number of the epoll fd.
    let inode_number_option: Option<u64> = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            match guard.get(epfd as usize)? {
                crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                    Some(inode.stat().inode_number)
                }
                _ => None,
            }
        })
    };

    let inode_number = match inode_number_option {
        Some(n) => n,
        None => return EBADF,
    };

    EPOLL_INSTANCE_TABLE.get_and_call(inode_number, |instance| {
        match operation {
            EPOLL_CTL_ADD => instance.ctl_add(watched_fd, event_mask, user_data),
            EPOLL_CTL_DEL => instance.ctl_del(watched_fd),
            EPOLL_CTL_MOD => instance.ctl_mod(watched_fd, event_mask, user_data),
            _ => EINVAL,
        }
    }).unwrap_or(EBADF)
}

/// `epoll_wait(epfd, events_ptr, maxevents, timeout_ms)` — wait for events.
///
/// Writes up to `maxevents` ready `EpollEvent` records to `events_ptr`.
/// Returns the number of events written, 0 on timeout, or a negative errno.
///
/// Blocking behaviour:
///   timeout_ms == 0:  return immediately (non-blocking check).
///   timeout_ms  > 0:  yield once per timer tick until deadline or event.
///   timeout_ms == -1: block indefinitely until at least one event is ready.
///
/// Reference: Linux `fs/eventpoll.c` `ep_poll()`.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn sys_epoll_wait(
    epfd: i32,
    events_ptr: u64,
    maxevents: i32,
    timeout_ms: i32,
) -> i64 {
    if epfd < 0 {
        return EBADF;
    }
    if maxevents <= 0 {
        return EINVAL;
    }
    let max = maxevents as usize;
    let event_size = core::mem::size_of::<crate::fs::epoll::EpollEvent>();
    if !validate_user_pointer(events_ptr, max * event_size) {
        return EINVAL;
    }

    // Identify the epoll instance by its inode number.
    let inode_number_option: Option<u64> = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            match guard.get(epfd as usize)? {
                crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                    Some(inode.stat().inode_number)
                }
                _ => None,
            }
        })
    };

    let inode_number = match inode_number_option {
        Some(n) => n,
        None => return EBADF,
    };

    // Calculate the deadline tick (for timeout_ms > 0).
    let deadline_tick: Option<u64> = if timeout_ms > 0 {
        let tick_interval_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
        let ticks_needed = ((timeout_ms as u64) + tick_interval_ms - 1) / tick_interval_ms;
        Some(crate::platform::qemu_virt::timer::current_tick()
            .saturating_add(ticks_needed))
    } else {
        None
    };

    // Temporary event buffer on the stack (max 64 events per call).
    // EpollEvent is Copy, so we can use a simple array initialiser.
    let mut event_buffer = [crate::fs::epoll::EpollEvent { events: 0, data: 0 }; 64];

    loop {
        let effective_max = max.min(event_buffer.len());

        let ready_count = EPOLL_INSTANCE_TABLE.get_and_call(
            inode_number,
            // SAFETY: single-core, IRQs disabled; slice length bounded by effective_max.
            |instance| unsafe {
                instance.collect_ready_events(&mut event_buffer[..effective_max], effective_max)
            },
        ).unwrap_or(0);

        if ready_count > 0 {
            // Copy ready events to the user buffer.
            let output_ptr = events_ptr as *mut crate::fs::epoll::EpollEvent;
            for event_index in 0..ready_count {
                // SAFETY: events_ptr is validated; index within bounds.
                core::ptr::write_unaligned(
                    output_ptr.add(event_index),
                    core::ptr::read_unaligned(&event_buffer[event_index]),
                );
            }
            return ready_count as i64;
        }

        if timeout_ms == 0 {
            return 0; // non-blocking — no events ready
        }
        if let Some(deadline) = deadline_tick {
            if crate::platform::qemu_virt::timer::current_tick() >= deadline {
                return 0; // timed out
            }
        }
        // Yield to allow other processes to run and (possibly) produce events.
        crate::scheduler::with_scheduler(|scheduler| { scheduler.schedule(); });
    }
}

// ---------------------------------------------------------------------------
// EpollInstanceTable — global registry keyed by inode number
// ---------------------------------------------------------------------------
//
// We cannot downcast `Arc<dyn Inode>` to `Arc<EpollInstance>` without the
// `Any` trait, which is not available in our no_std / no-core-introspection
// environment.  We maintain a parallel global table: `sys_epoll_create1`
// registers the new instance here; `sys_epoll_ctl` and `sys_epoll_wait` look
// it up by the inode number stored in the FileDescriptor.

use alloc::sync::Arc;

struct EpollInstanceTableInner {
    entries: alloc::vec::Vec<(u64, Arc<crate::fs::epoll::EpollInstance>)>,
}

struct EpollInstanceTable(core::cell::UnsafeCell<EpollInstanceTableInner>);

// SAFETY: single-core kernel; all kernel code runs with IRQs disabled.
unsafe impl Sync for EpollInstanceTable {}

static EPOLL_INSTANCE_TABLE: EpollInstanceTable = EpollInstanceTable(
    core::cell::UnsafeCell::new(EpollInstanceTableInner {
        entries: alloc::vec::Vec::new(),
    })
);

impl EpollInstanceTable {
    /// Register an EpollInstance under its inode number.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    unsafe fn register(
        &self,
        inode_number: u64,
        instance: Arc<crate::fs::epoll::EpollInstance>,
    ) {
        let inner = &mut *self.0.get();
        inner.entries.push((inode_number, instance));
    }

    /// Call `function` with the EpollInstance matching `inode_number`.
    ///
    /// Returns `Some(result)` if found, `None` if not found.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    unsafe fn get_and_call<F, R>(&self, inode_number: u64, function: F) -> Option<R>
    where
        F: FnOnce(&crate::fs::epoll::EpollInstance) -> R,
    {
        let inner = &*self.0.get();
        for (key, instance) in &inner.entries {
            if *key == inode_number {
                return Some(function(instance));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Phase 13 — POSIX threads primitives
// ---------------------------------------------------------------------------

/// Linux-compatible clone flag: share virtual memory (used for thread creation).
const CLONE_VM: u64 = 0x0000_0100;

/// Linux-compatible clone flag: thread group membership.
const CLONE_THREAD: u64 = 0x0001_0000;

/// Linux-compatible clone flag: set TLS base from arg2.
const CLONE_SETTLS: u64 = 0x0008_0000;

// ---------------------------------------------------------------------------
// sys_clone — create a new thread or process
// ---------------------------------------------------------------------------

/// `clone(flags, child_stack, tls) → child_tid | -errno`
///
/// Implements a subset of Linux clone(2):
///   - `CLONE_VM | CLONE_THREAD`: create a new thread sharing the page table.
///   - Neither flag set: fall back to fork semantics.
///
/// The child thread:
///   - Gets its own kernel stack and ExceptionFrame copy.
///   - Shares the parent's TTBR0 page table (same `Box<PageTable>` pointer is
///     *not* shared — see `clone_thread` in the scheduler for how the physical
///     root address is reused without double-ownership).
///   - Has `x0 = 0` in its return frame (thread-create convention: child sees 0).
///   - Starts with SP_EL0 = `child_stack` (must be a valid user stack).
///
/// # Safety
/// `frame` must be the current process's exception frame on the kernel stack.
unsafe fn sys_clone(frame: *mut ExceptionFrame, flags: u64, child_stack: u64, tls: u64) -> i64 {
    let is_thread_clone = (flags & CLONE_VM) != 0 && (flags & CLONE_THREAD) != 0;

    if !is_thread_clone {
        // Non-thread clone: fall back to fork().
        // SAFETY: frame is valid; IRQs are disabled at syscall entry.
        return match crate::scheduler::with_scheduler(|scheduler| scheduler.fork(frame)) {
            Ok(child_pid) => child_pid.index as i64,
            Err(_) => ENOMEM,
        };
    }

    // Thread clone: share address space with parent.
    // SAFETY: frame is valid; IRQs are disabled; clone_thread does not alias.
    let tls_requested = if (flags & CLONE_SETTLS) != 0 { tls } else { 0 };

    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.clone_thread(frame, child_stack, tls_requested) {
            Ok(child_pid) => child_pid.index as i64,
            Err(_) => ENOMEM,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_set_tls — set the TLS base address for the current thread
// ---------------------------------------------------------------------------

/// `set_tls(tls_base) → 0`
///
/// Stores `tls_base` in the current process's `tls_base` field and writes it
/// into `TPIDR_EL0` immediately.  Subsequent context switches will restore this
/// value via the scheduler's context-switch path.
///
/// # Safety
/// No user-pointer validation needed — TPIDR_EL0 is an opaque register
/// whose value is only interpreted by user-space TLS libraries.
unsafe fn sys_set_tls(tls_base: u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.tls_base = tls_base;
        }
    });
    // Write the new TLS base into the hardware register immediately so the
    // calling thread sees it without waiting for a context switch.
    //
    // Reference: ARM ARM DDI 0487 D13.2.116 "TPIDR_EL0".
    // SAFETY: TPIDR_EL0 is a user-accessible opaque register; writing it is
    // safe from EL1 at any time.
    core::arch::asm!(
        "msr tpidr_el0, {tls}",
        tls = in(reg) tls_base,
        options(nostack, nomem),
    );
    0
}

// ---------------------------------------------------------------------------
// sys_gettid — return the calling thread's TID
// ---------------------------------------------------------------------------

/// `gettid() → tid`
///
/// Returns the current thread's own TID (= `pid.index`), not the TGID.
/// Aligns with Linux: `gettid()` returns the per-thread ID; `getpid()` returns
/// the thread group leader's PID (tgid).
///
/// # Safety
/// No preconditions beyond the standard syscall invariant.
unsafe fn sys_gettid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_pid().index as i64
    })
}

// ---------------------------------------------------------------------------
// sys_framebuffer_map — map the boot framebuffer into the calling process
// ---------------------------------------------------------------------------

/// `framebuffer_map(out: *mut u64) → i64`
///
/// Maps the Limine-provided framebuffer into the calling process's address
/// space and writes descriptor fields into the 8-element u64 array at `out`:
///
///   out[0] = mapped virtual address (read-write, user accessible)
///   out[1] = width  (pixels)
///   out[2] = height (pixels)
///   out[3] = stride (bytes per row; ≥ width × bpp/8)
///   out[4] = bpp    (bits per pixel; always 32 in practice)
///   out[5] = red   info: (mask_size << 8) | mask_shift
///   out[6] = green info: (mask_size << 8) | mask_shift
///   out[7] = blue  info: (mask_size << 8) | mask_shift
///
/// Returns 0 on success, -EINVAL if the framebuffer is not yet available,
/// -ENOMEM if the mapping fails.
///
/// # Safety
/// `out` must be a valid user-space pointer to at least 64 bytes.
unsafe fn sys_framebuffer_map(out: *mut u64) -> i64 {
    if !validate_user_pointer(out as u64, 8 * core::mem::size_of::<u64>()) {
        return EINVAL;
    }

    // Only a process with CAP_DISPLAY may map the framebuffer.
    let authorized = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.capabilities & crate::process::CAP_DISPLAY != 0)
            .unwrap_or(false)
    });
    if !authorized {
        return EPERM;
    }

    let info = match crate::display::get() {
        Some(info) => info,
        None => return EINVAL,
    };

    let page_size = crate::memory::physical::read_page_size();
    let pages = ((info.size_bytes + page_size - 1) / page_size) as usize;

    let mapped_va = crate::memory::with_physical_allocator(|phys| {
        crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.map_physical_pages_for_current(
                info.phys_base,
                pages,
                page_size,
                phys,
            ) {
                Some(va) => va as i64,
                None => ENOMEM,
            }
        })
    });

    if mapped_va < 0 {
        return mapped_va;
    }

    // Write descriptor to user buffer.
    let out_slice = core::slice::from_raw_parts_mut(out, 8);
    out_slice[0] = mapped_va as u64;
    out_slice[1] = info.width;
    out_slice[2] = info.height;
    out_slice[3] = info.stride;
    out_slice[4] = info.bpp as u64;
    out_slice[5] = ((info.red_mask_size   as u64) << 8) | info.red_mask_shift   as u64;
    out_slice[6] = ((info.green_mask_size as u64) << 8) | info.green_mask_shift as u64;
    out_slice[7] = ((info.blue_mask_size  as u64) << 8) | info.blue_mask_shift  as u64;

    0
}

// ---------------------------------------------------------------------------
// sys_mkfifo — create a named pipe (FIFO) in the VFS
// ---------------------------------------------------------------------------

/// `mkfifo(path_ptr: u64, path_len: usize) → i64`
///
/// Creates a FIFO inode at the given absolute path in the VFS.
/// Subsequent `open()` calls on the same path will share the same ring buffer.
///
/// Returns 0 on success, negative errno on error.
///
/// # Safety
/// `path_ptr` must be a valid user-space pointer to `path_len` UTF-8 bytes.
unsafe fn sys_mkfifo(path_ptr: u64, path_len: usize) -> i64 {
    if path_ptr == 0 || path_len == 0 || path_len > 511 {
        return EINVAL;
    }
    if !validate_user_pointer(path_ptr, path_len) {
        return EINVAL;
    }

    let path_bytes = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Resolve the parent directory.
    let (parent_inode, file_name) = match crate::fs::vfs_resolve_parent(path) {
        Ok(pair) => pair,
        Err(error) => return error.to_errno(),
    };

    // Create the FifoInode and link it into the parent directory.
    let fifo_inode = crate::fs::fifo::FifoInode::new();
    match parent_inode.link_child(&file_name, fifo_inode) {
        Ok(()) => 0,
        Err(error) => error.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// Phase 16 — POSIX I/O multiplexing: select(2)
// ---------------------------------------------------------------------------

/// POSIX fd_set: a 1024-bit bitmap, one bit per file descriptor.
///
/// Layout: 16 × u64 words = 128 bytes = 1024 bits.
/// Bit position for fd N is: word[N / 64], bit (N % 64).
///
/// Reference: POSIX.1-2017 §2.14 "File Descriptor Sets".
#[repr(C)]
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
unsafe fn sys_select(
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

// ---------------------------------------------------------------------------
// sys_alarm — schedule SIGALRM delivery
// ---------------------------------------------------------------------------

/// Schedule delivery of SIGALRM after `seconds` seconds.
///
/// Returns the number of seconds remaining on any previously scheduled alarm
/// (0 if none was set).  Passing `seconds == 0` cancels any pending alarm.
///
/// The alarm fires at most once: after delivery, `alarm_deadline_tick` is
/// reset to 0 and the process must call `alarm()` again if another one is needed.
///
/// Reference: POSIX.1-2017 `alarm(2)`.
unsafe fn sys_alarm(seconds: u64) -> i64 {
    // TICK_INTERVAL_MS = 10 ms → 100 ticks per second.
    const TICKS_PER_SECOND: u64 =
        1_000 / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;

    let now_tick = crate::platform::qemu_virt::timer::current_tick();

    crate::scheduler::with_scheduler(|scheduler| {
        let process = match scheduler.current_process_mut() {
            Some(p) => p,
            None => return 0i64,
        };

        // Compute remaining seconds on any existing alarm.
        let remaining_seconds = if process.alarm_deadline_tick != 0
            && process.alarm_deadline_tick > now_tick
        {
            let remaining_ticks = process.alarm_deadline_tick - now_tick;
            // Round up: partial ticks count as a full second.
            (remaining_ticks + TICKS_PER_SECOND - 1) / TICKS_PER_SECOND
        } else {
            0
        };

        if seconds == 0 {
            // Cancel any pending alarm.
            process.alarm_deadline_tick = 0;
        } else {
            process.alarm_deadline_tick = now_tick + seconds * TICKS_PER_SECOND;
        }

        remaining_seconds as i64
    })
}

// ---------------------------------------------------------------------------
// sys_machine_reboot / sys_machine_poweroff — PSCI machine control
// ---------------------------------------------------------------------------

/// Reboot the machine via PSCI SYSTEM_RESET.
///
/// Uses the PSCI v0.2 / v1.0 `SYSTEM_RESET` function (ID 0x84000009)
/// invoked as an HVC call.  QEMU virt exposes PSCI via HVC by default.
///
/// Reference: ARM DEN0022D — Power State Coordination Interface (PSCI) §5.16.
///
/// # Safety
/// Terminates all execution unconditionally.
unsafe fn sys_machine_reboot() -> i64 {
    // PSCI_SYSTEM_RESET = 0x84000009 (SMC32 calling convention, function ID).
    // x0 = function ID; HVC #0.
    // MOVZ x0, #0x0009          → x0[15:0]  = 0x0009
    // MOVK x0, #0x8400, lsl #16 → x0[31:16] = 0x8400  → x0 = 0x84000009
    core::arch::asm!(
        "movz x0, #0x0009",
        "movk x0, #0x8400, lsl #16",
        "hvc #0",
        options(nostack, noreturn)
    );
}

/// Power off the machine via PSCI SYSTEM_OFF.
///
/// Uses the PSCI v0.2 / v1.0 `SYSTEM_OFF` function (ID 0x84000008)
/// invoked as an HVC call.
///
/// Reference: ARM DEN0022D — Power State Coordination Interface (PSCI) §5.15.
///
/// # Safety
/// Terminates all execution unconditionally.
unsafe fn sys_machine_poweroff() -> i64 {
    // PSCI_SYSTEM_OFF = 0x84000008.
    // MOVZ x0, #0x0008          → x0[15:0]  = 0x0008
    // MOVK x0, #0x8400, lsl #16 → x0[31:16] = 0x8400  → x0 = 0x84000008
    core::arch::asm!(
        "movz x0, #0x0008",
        "movk x0, #0x8400, lsl #16",
        "hvc #0",
        options(nostack, noreturn)
    );
}

// ---------------------------------------------------------------------------
// sys_getuid / sys_getgid / sys_geteuid / sys_getegid — POSIX UID/GID queries
//
// UIDs and GIDs are a POSIX compatibility shim.  The actual security mechanism
// in Bazzulto is the Binary Permission Model (see docs/features/Binary Permission Model.md).
// UIDs are needed so that standard tools (bash, coreutils, etc.) do not break.
//
// Default for user processes: uid=gid=euid=egid=1000.
// bzinit and kernel tasks run with uid=0.
//
// Reference: POSIX.1-2017 getuid(2), getgid(2), geteuid(2), getegid(2).
// ---------------------------------------------------------------------------

unsafe fn sys_getuid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.uid as i64)
            .unwrap_or(0)
    })
}

unsafe fn sys_getgid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.gid as i64)
            .unwrap_or(0)
    })
}

unsafe fn sys_geteuid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.euid as i64)
            .unwrap_or(0)
    })
}

unsafe fn sys_getegid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.egid as i64)
            .unwrap_or(0)
    })
}

/// POSIX setuid(2): set real and effective UID.
///
/// If euid == 0: may set uid/euid to any value.
/// Otherwise: may only set euid to uid (drop privileges).
///
/// Reference: POSIX.1-2017 setuid(2).
unsafe fn sys_setuid(new_uid: u32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            if process.euid == 0 {
                // Root: set both real and effective UID.
                process.uid  = new_uid;
                process.euid = new_uid;
                0
            } else if new_uid == process.uid {
                // Non-root: can set euid to real uid (no-op / drop saved set-uid).
                process.euid = new_uid;
                0
            } else {
                EPERM
            }
        } else {
            EPERM
        }
    })
}

/// POSIX setgid(2): set real and effective GID.
unsafe fn sys_setgid(new_gid: u32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            if process.euid == 0 {
                process.gid  = new_gid;
                process.egid = new_gid;
                0
            } else if new_gid == process.gid {
                process.egid = new_gid;
                0
            } else {
                EPERM
            }
        } else {
            EPERM
        }
    })
}

/// POSIX seteuid(2): set effective UID only.
unsafe fn sys_seteuid(new_euid: u32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            if process.euid == 0 || new_euid == process.uid {
                process.euid = new_euid;
                0
            } else {
                EPERM
            }
        } else {
            EPERM
        }
    })
}

/// POSIX setegid(2): set effective GID only.
unsafe fn sys_setegid(new_egid: u32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            if process.euid == 0 || new_egid == process.gid {
                process.egid = new_egid;
                0
            } else {
                EPERM
            }
        } else {
            EPERM
        }
    })
}

// ---------------------------------------------------------------------------
// sys_chmod / sys_fchmod / sys_chown / sys_fchown — POSIX file permission stubs
//
// Full permission enforcement requires the VFS inode to carry owner_uid/gid
// and mode bits (planned for a later pass).  For v1.0 these syscalls succeed
// silently so that coreutils and other tools do not error out.
// ---------------------------------------------------------------------------

unsafe fn sys_chmod(_path_ptr: u64, _path_len: usize, _mode: u32) -> i64 {
    // TODO: update inode mode bits when InodeStat carries owner info.
    0
}

unsafe fn sys_fchmod(_fd: i32, _mode: u32) -> i64 {
    // TODO: update inode mode bits.
    0
}

unsafe fn sys_chown(_path_ptr: u64, _path_len: usize, _new_uid: u32, _new_gid: u32) -> i64 {
    // TODO: update inode owner when InodeStat carries owner info.
    0
}

unsafe fn sys_fchown(_fd: i32, _new_uid: u32, _new_gid: u32) -> i64 {
    // TODO: update inode owner.
    0
}

// ---------------------------------------------------------------------------
// sys_mount — mount a filesystem at a VFS path
//
// Syscall number: 113.
//
// Arguments (x0–x5):
//   x0: source_ptr  — pointer to source path string (Bazzulto Path Model)
//   x1: source_len  — byte length of source path
//   x2: target_ptr  — pointer to target mountpoint path string
//   x3: target_len  — byte length of target path
//   x4: fstype_ptr  — pointer to filesystem type string ("fat32", "bafs", "tmpfs")
//   x5: fstype_len  — byte length of fstype
//
// Source path format (Bazzulto Path Model):
//   "//dev:diska:1/"  → disk index 0 (letter a=0), partition 1 (1-based → part_index 0)
//   "//dev:diskb:2/"  → disk index 1, partition 2 (part_index 1)
//   "//dev:diska/"    → disk index 0, bare disk (no partition table; part_index 0)
//
// Target path: native Bazzulto path ("//home:user/" or POSIX "/home/user").
// The target directory is created if it does not exist.
//
// Returns 0 on success, negative errno on failure.
//
// Required permission: ActionPermission::MountFilesystem.
// ---------------------------------------------------------------------------

unsafe fn sys_mount(
    source_ptr: u64,
    source_len: usize,
    target_ptr: u64,
    target_len: usize,
    fstype_ptr: u64,
    fstype_len: usize,
) -> i64 {
    const EPERM:   i64 = -1;
    const ENODEV:  i64 = -19;
    const EINVAL:  i64 = -22;
    const ENOENT:  i64 = -2;
    const ENOMEM:  i64 = -12;

    // --- Permission check ---------------------------------------------------
    let has_permission = crate::scheduler::with_scheduler(|s| {
        s.current_process().map(|p| {
            crate::permission::check_action_permission(
                &p.granted_actions,
                crate::permission::ActionPermission::MountFilesystem,
            ).is_ok()
        }).unwrap_or(false)
    });
    if !has_permission {
        return EPERM;
    }

    // --- Read user strings --------------------------------------------------
    if source_len == 0 || source_len > 256 { return EINVAL; }
    if target_len == 0 || target_len > 256 { return EINVAL; }
    if fstype_len == 0 || fstype_len > 16  { return EINVAL; }

    let source_bytes = core::slice::from_raw_parts(source_ptr as *const u8, source_len);
    let target_bytes = core::slice::from_raw_parts(target_ptr as *const u8, target_len);
    let fstype_bytes = core::slice::from_raw_parts(fstype_ptr as *const u8, fstype_len);

    let source = match core::str::from_utf8(source_bytes) { Ok(s) => s, Err(_) => return EINVAL };
    let target = match core::str::from_utf8(target_bytes) { Ok(s) => s, Err(_) => return EINVAL };
    let fstype = match core::str::from_utf8(fstype_bytes) { Ok(s) => s, Err(_) => return EINVAL };

    // --- Parse source: "//dev:disk{x}:{y}/" --------------------------------
    // Strip "//dev:disk" prefix.
    let rest = match source.strip_prefix("//dev:disk") {
        Some(r) => r,
        None    => return EINVAL,
    };
    // First character is the disk letter ('a'=0, 'b'=1, ...).
    let letter = match rest.as_bytes().first().copied() {
        Some(ch) if ch.is_ascii_lowercase() => ch,
        _ => return EINVAL,
    };
    let disk_index = (letter - b'a') as usize;
    let after_letter = &rest[1..]; // ":1/" or "/"

    // Partition number: optional ":{N}" suffix.  Absent means bare disk (part 1).
    let part_number_1based: usize = if let Some(colon_rest) = after_letter.strip_prefix(':') {
        // Parse the digits before the trailing '/'.
        let digits = colon_rest.trim_end_matches('/');
        match digits.parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => return EINVAL,
        }
    } else {
        1 // bare disk shorthand → partition 1
    };
    let target_part_index = part_number_1based - 1; // convert to 0-based

    // --- Find and mount the partition ---------------------------------------
    let disk = match crate::hal::disk::get_disk(disk_index) {
        Some(d) => d,
        None    => return ENODEV,
    };

    let partitions = crate::fs::partition::enumerate_partitions(disk, disk_index);
    let partition = match partitions.into_iter().find(|p| p.part_index == target_part_index) {
        Some(p) => p,
        None    => return ENODEV,
    };

    // Normalize the target path for the VFS mount table.
    // "//home:user/" → "/home/user" (simple prefix strip + colon → slash replacement).
    // If already a POSIX path ("/home/user"), use as-is.
    let mut posix_target_buf = [0u8; 256];
    let posix_target: &str = if target.starts_with("//") {
        // Bazzulto path model: strip leading '/' → "/home:user/" → replace ':' with '/'
        // then strip trailing '/'.
        let inner = &target[1..]; // "/home:user/"
        let mut out_len = 0usize;
        for b in inner.as_bytes() {
            let ch = if *b == b':' { b'/' } else { *b };
            if out_len >= posix_target_buf.len() { return EINVAL; }
            posix_target_buf[out_len] = ch;
            out_len += 1;
        }
        // Strip trailing slash unless it's just "/".
        while out_len > 1 && posix_target_buf[out_len - 1] == b'/' {
            out_len -= 1;
        }
        match core::str::from_utf8(&posix_target_buf[..out_len]) {
            Ok(s) => s,
            Err(_) => return EINVAL,
        }
    } else {
        target
    };

    // Ensure the mountpoint directory exists (create if needed).
    if let Ok((parent, name)) = crate::fs::vfs_resolve_parent(posix_target) {
        let _ = parent.mkdir(&name);
    }

    // Probe and mount according to the requested filesystem type.
    if fstype.eq_ignore_ascii_case("fat32") {
        if !partition.is_fat32_candidate() {
            return EINVAL;
        }
        let volume = match crate::fs::fat32::fat32_init_partition(partition.disk, partition.start_lba) {
            Some(v) => v,
            None    => return ENODEV,
        };
        let root_inode = match crate::fs::fat32::fat32_root_inode(volume) {
            Some(i) => i,
            None    => return ENOMEM,
        };
        crate::fs::vfs_mount(posix_target, root_inode, source, "fat32");
        0
    } else if fstype.eq_ignore_ascii_case("bafs") {
        if !crate::fs::bafs_driver::bafs_probe(&partition.disk, partition.start_lba) {
            return ENODEV;
        }
        let root_inode = match crate::fs::bafs_driver::bafs_mount_partition(partition.disk, partition.start_lba) {
            Some(i) => i,
            None    => return ENODEV,
        };
        crate::fs::vfs_mount(posix_target, root_inode, source, "bafs");
        0
    } else {
        EINVAL
    }
}

// ---------------------------------------------------------------------------
// sys_getmounts — enumerate mounted filesystems
// ---------------------------------------------------------------------------
//
// Syscall number: 114.
//
// Serialises all VFS mount entries into a flat byte buffer for userspace.
//
// Buffer format (packed, variable length):
//   For each mount entry:
//     [0]     u8  — mountpoint length  (bytes)
//     [1..n]  u8* — mountpoint path    (not NUL-terminated)
//     [n]     u8  — source length      (bytes, 0 for virtual filesystems)
//     [n+1..] u8* — source path        (not NUL-terminated)
//     [...]   u8  — fstype length      (bytes)
//     [...]   u8* — fstype string      (not NUL-terminated)
//     [...]   u64 — total 512-blocks   (little-endian)
//     [...]   u64 — free 512-blocks    (little-endian)
//
// Returns the total number of bytes written on success, or -EINVAL / -ENOMEM.
// Pass buf_ptr=0 and buf_len=0 to query the required buffer size.
//
// The caller should allocate a buffer of the returned size and call again.
//
// Reference: Linux /proc/mounts format; POSIX statvfs(3).
unsafe fn sys_getmounts(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    use alloc::vec::Vec;

    // Accumulate serialised entries into a heap buffer, then copy to user.
    let mut serialised: Vec<u8> = Vec::new();

    crate::fs::vfs_for_each_mount(|mountpoint, source, fstype, root_inode| {
        // Compute 512-block statistics.
        // For FAT32 we can obtain real stats via the fat32_volume_stats helper.
        // For other filesystem types report 0 (unknown) — userspace shows "-".
        let (total_blocks, free_blocks): (u64, u64) =
            if fstype == "fat32" {
                // Downcast root_inode to Fat32DirInode to reach the volume Arc.
                // We expose a helper on the inode trait for this purpose.
                if let Some(stats) = root_inode.fs_stats() {
                    stats
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

        // Serialise the entry.
        let mp_bytes = mountpoint.as_bytes();
        let src_bytes = source.as_bytes();
        let fs_bytes  = fstype.as_bytes();

        // Lengths are capped at 255 bytes (single u8 field).
        let mp_len  = mp_bytes.len().min(255) as u8;
        let src_len = src_bytes.len().min(255) as u8;
        let fs_len  = fs_bytes.len().min(255)  as u8;

        serialised.push(mp_len);
        serialised.extend_from_slice(&mp_bytes[..mp_len as usize]);
        serialised.push(src_len);
        serialised.extend_from_slice(&src_bytes[..src_len as usize]);
        serialised.push(fs_len);
        serialised.extend_from_slice(&fs_bytes[..fs_len as usize]);
        serialised.extend_from_slice(&total_blocks.to_le_bytes());
        serialised.extend_from_slice(&free_blocks.to_le_bytes());
    });

    let total_len = serialised.len();

    // Query mode: return required size without writing.
    if buf_ptr.is_null() || buf_len == 0 {
        return total_len as i64;
    }

    if !validate_user_pointer(buf_ptr as u64, buf_len) {
        return EINVAL;
    }

    if buf_len < total_len {
        // Buffer too small — return required size as positive so caller can retry.
        return total_len as i64;
    }

    // Pre-fault all demand pages in the user buffer before writing from EL1.
    // EL1 data aborts do not go through the demand-paging handler; writing to
    // an unmapped demand page from kernel context would halt the kernel.
    if !crate::memory::fault_in_user_write_pages(buf_ptr as u64, total_len) {
        return EINVAL;
    }

    // Copy serialised data to userspace buffer.
    core::ptr::copy_nonoverlapping(serialised.as_ptr(), buf_ptr, total_len);

    total_len as i64
}
