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
                  SLOT_CREAT, SLOT_UNLINK, SLOT_FSTAT, SLOT_SETFGPID,
                  SLOT_GETRUSAGE, SLOT_GETCWD, SLOT_MKDIR, SLOT_GETDENTS64,
                  SLOT_CHDIR, SLOT_FRAMEBUFFER_MAP,
                  SLOT_UMASK, SLOT_SIGALTSTACK, SLOT_ALARM,
                  SLOT_MACHINE_REBOOT, SLOT_MACHINE_POWEROFF,
                  SLOT_MOUNT, SLOT_GETMOUNTS,
                  SLOT_UNAME, SLOT_SYSINFO,
                  SLOT_GETUID, SLOT_GETGID,
                  SLOT_TCGETATTR, SLOT_TCSETATTR};

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

macro_rules! vdso_call6 {
    ($slot:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr) => {{
        let stub = vdso_slot_va($slot) as *const ();
        let f: unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> i64 = unsafe { core::mem::transmute(stub) };
        unsafe { f($a0 as u64, $a1 as u64, $a2 as u64, $a3 as u64, $a4 as u64, $a5 as u64) }
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

/// Open a file. `name` must be a NUL-terminated C string.
/// `flags` is a combination of O_* constants; `mode` is the permission bits
/// (only meaningful when O_CREAT is set).
/// Returns fd >= 0 on success, negative errno on failure.
#[inline]
pub fn raw_open(name: *const u8, flags: i32, mode: u32) -> i64 {
    vdso_call3!(SLOT_OPEN, name, flags, mode)
}

#[inline]
pub fn raw_close(fd: i32) -> i64 {
    vdso_call1!(SLOT_CLOSE, fd)
}

#[inline]
pub fn raw_seek(fd: i32, offset: i64, whence: i32) -> i64 {
    vdso_call3!(SLOT_SEEK, fd, offset as u64, whence)
}

/// Spawn a child process from a ramfs path. `name` must be NUL-terminated.
/// Returns child PID on success, negative errno on failure.
#[inline]
pub fn raw_spawn(name: *const u8) -> i64 {
    vdso_call2!(SLOT_SPAWN, name, 0u64)
}

/// Like `raw_spawn` but grants `capability_mask` to the new process.
///
/// The caller must hold `CAP_SETCAP` and each capability in the mask.
/// `name` must be NUL-terminated.
/// Returns the child PID on success, negative errno on failure.
#[inline]
pub fn raw_spawn_with_capabilities(
    name:             *const u8,
    capability_mask:  u64,
) -> i64 {
    vdso_call2!(SLOT_SPAWN, name, capability_mask)
}

/// List ramfs entries. Copies entry name into `buf`. Returns name length or < 0.
#[inline]
pub fn raw_list(buf: *mut u8, buf_len: usize) -> i64 {
    vdso_call2!(SLOT_LIST, buf, buf_len)
}

/// Wait for a child process. `pid = -1` waits for any child.
/// `options`: 0 to block, 1 (WNOHANG) to return 0 immediately if no child exited.
/// Returns child PID; writes exit status to `*status_out` if non-null.
#[inline]
pub fn raw_wait(pid: i32, status_out: *mut i32, options: i32) -> i64 {
    vdso_call4!(SLOT_WAIT, pid, status_out, options, 0u64)
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

/// Replace current process image. `name` must be NUL-terminated.
///
/// `argv` is a NULL-terminated array of pointers to NUL-terminated strings
/// (POSIX execv convention). Pass a pointer to a null pointer for no args.
/// `envp` is a NULL-terminated array of `KEY=VALUE\0` NUL-terminated strings.
/// Pass a pointer to a null pointer to inherit no environment.
#[inline]
pub fn raw_exec(name: *const u8, argv: *const *const u8, envp: *const *const u8) -> i64 {
    vdso_call3!(SLOT_EXEC, name, argv, envp)
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

/// Install or query a signal action. `new_act` and `old_act` each point to
/// a 152-byte rt_sigaction struct: {sa_handler:u64, sa_flags:u64,
/// sa_restorer:u64, sa_mask:[u64;16]}. Pass null to skip either.
/// `sigsetsize` must be 8 (size of the mask in bytes that the kernel reads).
#[inline]
pub fn raw_sigaction(signal_number: i32, new_act: *const u8, old_act: *mut u8) -> i64 {
    vdso_call4!(SLOT_SIGACTION, signal_number, new_act, old_act, 8u64)
}

#[inline]
pub fn raw_kill(pid: i32, signal_number: i32) -> i64 {
    vdso_call2!(SLOT_KILL, pid, signal_number)
}

/// Create or truncate a file. `name` must be NUL-terminated. `mode` sets
/// permission bits (umask applied by kernel). Returns fd on success.
#[inline]
pub fn raw_creat(name: *const u8, mode: u32) -> i64 {
    vdso_call2!(SLOT_CREAT, name, mode)
}

/// Unlink (delete) a file. `name` must be NUL-terminated.
#[inline]
pub fn raw_unlink(name: *const u8) -> i64 {
    vdso_call1!(SLOT_UNLINK, name)
}

/// Get file metadata by file descriptor.
///
/// On success writes a 128-byte Linux stat64 struct into `stat_buf`.
/// Key offsets: size at +40 (u64), mode at +16 (u32), nlink at +20 (u32),
/// ino at +8 (u64), blksize at +48 (u64), blocks at +56 (u64).
///
/// Returns 0 on success or a negative errno on failure.
/// `stat_buf` must point to at least 128 bytes of writable memory.
#[inline]
pub fn raw_fstat(fd: i32, stat_buf: *mut u8) -> i64 {
    vdso_call2!(SLOT_FSTAT, fd, stat_buf)
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

/// Create directory at `path` with the given `mode`. `path` must be NUL-terminated.
///
/// Returns 0 on success or a negative errno on failure.
#[inline]
pub fn raw_mkdir(path: *const u8, mode: u32) -> i64 {
    vdso_call2!(SLOT_MKDIR, path, mode)
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

/// Change the current working directory. `path` must be NUL-terminated.
///
/// Returns 0 on success or a negative errno on failure.
#[inline]
pub fn raw_chdir(path: *const u8) -> i64 {
    vdso_call1!(SLOT_CHDIR, path)
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

/// Mount a filesystem. All three string arguments must be NUL-terminated.
///
/// `source`: Bazzulto Path Model device path (e.g. `"//dev:diskb:1/"`).
/// `target`: mountpoint path (e.g. `"/home/user"`).
/// `fstype`: filesystem type (`"fat32"`, `"bafs"`, `"tmpfs"`).
///
/// Returns 0 on success or a negative errno on failure.
#[inline]
pub fn raw_mount(source: *const u8, target: *const u8, fstype: *const u8) -> i64 {
    vdso_call3!(SLOT_MOUNT, source, target, fstype)
}

/// Enumerate mounted filesystems into `buf`.
///
/// Pass `buf_ptr = null` and `buf_len = 0` to query the required buffer size.
/// Returns the total bytes written (or required) on success, or negative errno.
///
/// Buffer format: see `sys_getmounts` in the kernel syscall module.
#[inline]
pub fn raw_getmounts(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    vdso_call2!(SLOT_GETMOUNTS, buf_ptr, buf_len)
}

/// Fill a POSIX-compatible `utsname` buffer with OS identity strings.
///
/// Buffer layout: 6 fields × 65 bytes, each NUL-terminated:
///   [0..65]    sysname   — OS name, e.g. "Bazzulto"
///   [65..130]  nodename  — hostname, e.g. "bazzulto"
///   [130..195] release   — kernel release, e.g. "0.1.0"
///   [195..260] version   — build description, e.g. "Bazzulto 0.1.0 (AArch64)"
///   [260..325] machine   — ISA, e.g. "aarch64"
///   [325..390] domainname — NIS domain name (empty string)
///
/// Returns 0 on success or a negative errno.
#[inline]
pub fn raw_uname(buf: *mut u8) -> i64 {
    vdso_call1!(SLOT_UNAME, buf)
}

/// Fill a sysinfo buffer with system-wide statistics.
///
/// Buffer layout: 4 × u64 (little-endian, naturally aligned):
///   [0]  uptime_seconds — monotonic seconds since boot
///   [1]  total_ram      — total physical RAM in bytes
///   [2]  free_ram       — free physical RAM in bytes
///   [3]  process_count  — number of live processes
///
/// Returns 0 on success or a negative errno.
#[inline]
pub fn raw_sysinfo(buf: *mut u64) -> i64 {
    vdso_call1!(SLOT_SYSINFO, buf)
}

// ---------------------------------------------------------------------------
// Identity syscalls — all now within vDSO range (200 slots)
// ---------------------------------------------------------------------------

use crate::vdso::{SLOT_GETEUID, SLOT_GETEGID, SLOT_SETUID, SLOT_SETGID,
                  SLOT_SYMLINK, SLOT_GETGROUPS};

/// Create a symbolic link at `linkpath` pointing to `target`.
/// Both paths must be NUL-terminated.
/// Returns 0 on success or a negative errno.
#[inline]
pub fn raw_symlink(target: *const u8, linkpath: *const u8) -> i64 {
    vdso_call2!(SLOT_SYMLINK, target, linkpath)
}

/// Set the real and effective user ID of the calling process.
/// Privileged (euid==0): sets uid, euid, suid.
/// Unprivileged: may only set euid to uid or suid.
#[inline]
pub fn raw_setuid(uid: u32) -> i64 {
    vdso_call1!(SLOT_SETUID, uid)
}

/// Set the real and effective group ID.  Same privilege rules as setuid.
#[inline]
pub fn raw_setgid(gid: u32) -> i64 {
    vdso_call1!(SLOT_SETGID, gid)
}

/// Get the real user ID.
#[inline]
pub fn raw_getuid() -> u32 {
    vdso_call0!(SLOT_GETUID) as u32
}

/// Get the real group ID.
#[inline]
pub fn raw_getgid() -> u32 {
    vdso_call0!(SLOT_GETGID) as u32
}

/// Get the effective user ID.
#[inline]
pub fn raw_geteuid() -> u32 {
    vdso_call0!(SLOT_GETEUID) as u32
}

/// Get the effective group ID.
#[inline]
pub fn raw_getegid() -> u32 {
    vdso_call0!(SLOT_GETEGID) as u32
}

/// Get supplementary group IDs.
/// If `size` == 0, returns the number of supplementary groups.
/// Otherwise fills `list[0..size]` and returns the count.
#[inline]
pub fn raw_getgroups(size: i32, list: *mut u32) -> i64 {
    vdso_call2!(SLOT_GETGROUPS, size, list)
}

/// Get terminal attributes.  `termios_ptr` must point to a 48-byte Termios struct.
#[inline]
pub fn raw_tcgetattr(fd: i32, termios_ptr: *mut u8) -> i64 {
    vdso_call2!(SLOT_TCGETATTR, fd, termios_ptr)
}

/// Set terminal attributes.  `termios_ptr` must point to a 48-byte Termios struct.
#[inline]
pub fn raw_tcsetattr(fd: i32, optional_actions: i32, termios_ptr: *const u8) -> i64 {
    vdso_call3!(SLOT_TCSETATTR, fd, optional_actions, termios_ptr)
}
