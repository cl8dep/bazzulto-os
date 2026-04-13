//! Raw vDSO call trampolines.
//!
//! Each function loads the vDSO slot virtual address and branches to it.
//! The SVC instruction lives in the kernel-managed vDSO page — not here.
//! Arguments follow AAPCS64 (x0–x5); return value in x0.

use crate::vdso::{vdso_slot_va, SLOT_EXIT, SLOT_WRITE, SLOT_READ, SLOT_YIELD,
                  SLOT_OPEN, SLOT_CLOSE, SLOT_SEEK, SLOT_SPAWN, SLOT_LIST,
                  SLOT_WAIT, SLOT_PIPE, SLOT_DUP, SLOT_DUP2, SLOT_MMAP,
                  SLOT_MUNMAP, SLOT_FORK, SLOT_EXEC, SLOT_GETPID, SLOT_GETPPID,
                  SLOT_CLOCK_GETTIME, SLOT_NANOSLEEP, SLOT_SIGACTION, SLOT_KILL,
                  SLOT_CREAT, SLOT_UNLINK, SLOT_SETFGPID, SLOT_GETRUSAGE,
                  SLOT_GETCWD, SLOT_MKDIR, SLOT_GETDENTS64, SLOT_CHDIR,
                  SLOT_FRAMEBUFFER_MAP,
                  SLOT_UMASK, SLOT_SIGALTSTACK, SLOT_ALARM,
                  SLOT_MACHINE_REBOOT, SLOT_MACHINE_POWEROFF};

// Branch into a vDSO slot that takes 0 arguments and returns i64.
macro_rules! vdso_call0 {
    ($slot:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn() -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f() }
    }};
}

// Branch into a vDSO slot with 1 argument.
macro_rules! vdso_call1 {
    ($slot:expr, $a0:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn(u64) -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f($a0 as u64) }
    }};
}

macro_rules! vdso_call2 {
    ($slot:expr, $a0:expr, $a1:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn(u64, u64) -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f($a0 as u64, $a1 as u64) }
    }};
}

macro_rules! vdso_call3 {
    ($slot:expr, $a0:expr, $a1:expr, $a2:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn(u64, u64, u64) -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f($a0 as u64, $a1 as u64, $a2 as u64) }
    }};
}

macro_rules! vdso_call4 {
    ($slot:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn(u64, u64, u64, u64) -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f($a0 as u64, $a1 as u64, $a2 as u64, $a3 as u64) }
    }};
}

// ---------------------------------------------------------------------------
// Individual syscall wrappers
// ---------------------------------------------------------------------------

#[inline]
pub fn raw_exit(code: i32) -> ! {
    vdso_call1!(SLOT_EXIT, code);
    loop {}  // unreachable — kernel never returns from exit
}

#[inline]
pub fn raw_write(fd: i32, buf: *const u8, len: usize) -> i64 {
    vdso_call3!(SLOT_WRITE, fd, buf, len)
}

#[inline]
pub fn raw_read(fd: i32, buf: *mut u8, len: usize) -> i64 {
    vdso_call3!(SLOT_READ, fd, buf, len)
}

#[inline]
pub fn raw_yield() -> i64 {
    vdso_call0!(SLOT_YIELD)
}

/// Open a file by path. `name` must be a valid UTF-8 byte slice (not NUL-terminated).
/// Returns fd >= 0 on success, negative errno on failure.
#[inline]
pub fn raw_open(name: *const u8, name_len: usize) -> i64 {
    vdso_call2!(SLOT_OPEN, name, name_len)
}

#[inline]
pub fn raw_close(fd: i32) -> i64 {
    vdso_call1!(SLOT_CLOSE, fd)
}

#[inline]
pub fn raw_seek(fd: i32, offset: i64, whence: i32) -> i64 {
    vdso_call3!(SLOT_SEEK, fd, offset as u64, whence)
}

/// Spawn a child process from a ramfs path. Returns child PID on success.
#[inline]
pub fn raw_spawn(name: *const u8, name_len: usize) -> i64 {
    vdso_call3!(SLOT_SPAWN, name, name_len, 0u64)
}

/// Like `raw_spawn` but grants `capability_mask` to the new process.
///
/// The caller must hold `CAP_SETCAP` and each capability in the mask.
/// Returns the child PID on success, negative errno on failure.
#[inline]
pub fn raw_spawn_with_capabilities(
    name:             *const u8,
    name_len:         usize,
    capability_mask:  u64,
) -> i64 {
    vdso_call3!(SLOT_SPAWN, name, name_len, capability_mask)
}

/// List ramfs entries. Copies entry name into `buf`. Returns name length or < 0.
#[inline]
pub fn raw_list(buf: *mut u8, buf_len: usize) -> i64 {
    vdso_call2!(SLOT_LIST, buf, buf_len)
}

/// Wait for a child process. `pid = -1` waits for any child.
/// Returns child PID; writes exit status to `*status_out` if non-null.
#[inline]
pub fn raw_wait(pid: i32, status_out: *mut i32) -> i64 {
    vdso_call2!(SLOT_WAIT, pid, status_out)
}

/// Create a pipe. Writes two fds into `fd_pair[0]` (read) and `fd_pair[1]` (write).
#[inline]
pub fn raw_pipe(fd_pair: *mut i32) -> i64 {
    vdso_call1!(SLOT_PIPE, fd_pair)
}

#[inline]
pub fn raw_dup(source_fd: i32) -> i64 {
    vdso_call1!(SLOT_DUP, source_fd)
}

#[inline]
pub fn raw_dup2(source_fd: i32, dest_fd: i32) -> i64 {
    vdso_call2!(SLOT_DUP2, source_fd, dest_fd)
}

/// Anonymous mmap. Returns base address on success, negative errno on failure.
#[inline]
pub fn raw_mmap(addr: u64, length: u64, prot: i32, flags: i32) -> i64 {
    vdso_call4!(SLOT_MMAP, addr, length, prot, flags)
}

#[inline]
pub fn raw_munmap(addr: u64, length: u64) -> i64 {
    vdso_call2!(SLOT_MUNMAP, addr, length)
}

#[inline]
pub fn raw_fork() -> i64 {
    // fork() takes no arguments; the kernel reads the exception frame directly.
    vdso_call0!(SLOT_FORK)
}

/// Replace current process image. `name` is the ramfs path.
///
/// `argv_flat` is a flat buffer of NUL-separated argument strings
/// (e.g. `"cat\0file.txt\0"`). Pass `core::ptr::null()` and `0` if no
/// arguments are needed.
#[inline]
pub fn raw_exec(name: *const u8, name_len: usize, argv_flat: *const u8, argv_len: usize) -> i64 {
    vdso_call4!(SLOT_EXEC, name, name_len, argv_flat, argv_len)
}

#[inline]
pub fn raw_getpid() -> i64 {
    vdso_call0!(SLOT_GETPID)
}

#[inline]
pub fn raw_getppid() -> i64 {
    vdso_call0!(SLOT_GETPPID)
}

#[inline]
pub fn raw_clock_gettime(clock_id: i32, timespec_ptr: *mut u64) -> i64 {
    vdso_call2!(SLOT_CLOCK_GETTIME, clock_id, timespec_ptr)
}

#[inline]
pub fn raw_nanosleep(req: *const u64) -> i64 {
    vdso_call1!(SLOT_NANOSLEEP, req)
}

#[inline]
pub fn raw_sigaction(signal_number: i32, handler_va: u64, old_handler: *mut u64) -> i64 {
    vdso_call3!(SLOT_SIGACTION, signal_number, handler_va, old_handler)
}

#[inline]
pub fn raw_kill(pid: i32, signal_number: i32) -> i64 {
    vdso_call2!(SLOT_KILL, pid, signal_number)
}

#[inline]
pub fn raw_creat(name: *const u8, name_len: usize) -> i64 {
    vdso_call3!(SLOT_CREAT, name, name_len, 0usize) // flags=0 → truncate
}

/// Open or create a file for writing WITHOUT truncating (for `>>` append).
#[inline]
pub fn raw_creat_append(name: *const u8, name_len: usize) -> i64 {
    vdso_call3!(SLOT_CREAT, name, name_len, 1usize) // flags=1 → no truncate
}

#[inline]
pub fn raw_unlink(name: *const u8, name_len: usize) -> i64 {
    vdso_call2!(SLOT_UNLINK, name, name_len)
}

#[inline]
pub fn raw_setfgpid(pid: i32) -> i64 {
    vdso_call1!(SLOT_SETFGPID, pid)
}

/// Fill `buf` with the current working directory path (null-terminated string).
///
/// Returns the number of bytes written (including null terminator) on success,
/// or a negative errno on failure.
#[inline]
pub fn raw_getcwd(buf: *mut u8, buf_len: usize) -> i64 {
    vdso_call2!(SLOT_GETCWD, buf, buf_len)
}

/// Create directory at `path` with the given `mode`.
///
/// Returns 0 on success or a negative errno on failure.
#[inline]
pub fn raw_mkdir(path: *const u8, path_len: usize, mode: u32) -> i64 {
    vdso_call3!(SLOT_MKDIR, path, path_len, mode)
}

/// Read directory entries into `buf`.
///
/// Returns bytes written, 0 at end of directory, or negative errno.
/// Each entry is a `linux_dirent64`-compatible struct: ino(u64), off(u64),
/// reclen(u16), type(u8), name(null-terminated).
#[inline]
pub fn raw_getdents64(fd: i32, buf: *mut u8, buf_len: usize) -> i64 {
    vdso_call3!(SLOT_GETDENTS64, fd, buf, buf_len)
}

/// Change the current working directory.
///
/// Returns 0 on success or a negative errno on failure.
#[inline]
pub fn raw_chdir(path: *const u8, path_len: usize) -> i64 {
    vdso_call2!(SLOT_CHDIR, path, path_len)
}

/// Retrieve resource usage for the current process or its children.
///
/// `who`: 0 = RUSAGE_SELF, 1 = RUSAGE_CHILDREN.
/// `buf` must point to a `struct rusage`-compatible buffer (144 bytes on AArch64).
/// Returns 0 on success or a negative errno.
#[inline]
pub fn raw_getrusage(who: i32, buf: *mut u8) -> i64 {
    vdso_call2!(SLOT_GETRUSAGE, who, buf)
}

#[inline]
pub fn raw_framebuffer_map(out: *mut u64) -> i64 {
    vdso_call1!(SLOT_FRAMEBUFFER_MAP, out)
}

/// Set the file-creation mode mask; returns the previous umask.
#[inline]
pub fn raw_umask(mask: u32) -> u32 {
    vdso_call1!(SLOT_UMASK, mask) as u32
}

/// Set or query the alternate signal stack.
/// `new_stack` and `old_stack` are pointers to `bz_stack_t`-compatible structs
/// (ss_sp: u64, ss_flags: u32 + 4 pad, ss_size: u64 — 24 bytes).
/// Pass 0 for either to skip that operation.
/// Returns 0 on success or a negative errno.
#[inline]
pub fn raw_sigaltstack(new_stack: *const u8, old_stack: *mut u8) -> i64 {
    vdso_call2!(SLOT_SIGALTSTACK, new_stack, old_stack)
}

/// Schedule SIGALRM delivery after `seconds` seconds.
/// Returns the number of seconds remaining on any previously scheduled alarm,
/// or 0 if none was set.  Passing 0 cancels any pending alarm.
#[inline]
pub fn raw_alarm(seconds: u64) -> u64 {
    vdso_call1!(SLOT_ALARM, seconds) as u64
}

/// Reboot the machine via PSCI SYSTEM_RESET. Does not return.
#[inline]
pub fn raw_machine_reboot() -> ! {
    vdso_call0!(SLOT_MACHINE_REBOOT);
    loop {}
}

/// Power off the machine via PSCI SYSTEM_OFF. Does not return.
#[inline]
pub fn raw_machine_poweroff() -> ! {
    vdso_call0!(SLOT_MACHINE_POWEROFF);
    loop {}
}
