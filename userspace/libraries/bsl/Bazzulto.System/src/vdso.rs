//! vDSO trampoline layer.
//!
//! The kernel maps a read-only page at `VDSO_BASE` into every process. That
//! page contains `svc #N; ret` stubs — one per syscall slot (16 bytes each).
//! We branch into the page instead of encoding SVC immediates in userspace
//! binaries, so the kernel can renumber syscalls without breaking compiled ELFs.
//!
//! # Slot assignment (stable forever — never reorder, never reuse)
//!
//! Slot IDs map to Bazzulto syscall *functions*, not kernel SVC numbers. The
//! kernel writes the current SVC number into each slot at boot.
//!
//!   Slot  0 → exit
//!   Slot  1 → write
//!   Slot  2 → read
//!   Slot  3 → yield
//!   Slot  4 → open
//!   Slot  5 → close
//!   Slot  6 → seek
//!   Slot  7 → spawn
//!   Slot  8 → list
//!   Slot  9 → wait
//!   Slot 10 → pipe
//!   Slot 11 → dup
//!   Slot 12 → dup2
//!   Slot 13 → mmap
//!   Slot 14 → munmap
//!   Slot 15 → fork
//!   Slot 16 → exec
//!   Slot 17 → getpid
//!   Slot 18 → getppid
//!   Slot 19 → clock_gettime
//!   Slot 20 → nanosleep
//!   Slot 21 → sigaction
//!   Slot 22 → kill
//!   Slot 23 → sigreturn
//!   Slot 24 → creat
//!   Slot 25 → unlink
//!   Slot 26 → fstat
//!   Slot 27 → setfgpid
//!   Slot 28 → disk_info

/// Virtual address at which the kernel maps the vDSO page.
/// This constant is permanent — changing it breaks all compiled binaries.
pub const VDSO_BASE: usize = 0x1000;

/// Bytes per vDSO slot: `svc #N` (4 bytes) + `ret` (4 bytes).
pub const VDSO_SLOT_SIZE: usize = 8;

/// Compute the virtual address of a vDSO slot.
#[inline(always)]
pub const fn vdso_slot_va(slot: usize) -> usize {
    VDSO_BASE + slot * VDSO_SLOT_SIZE
}

// Slot ID constants — use these in raw.rs trampolines.
pub const SLOT_EXIT:          usize = 0;
pub const SLOT_WRITE:         usize = 1;
pub const SLOT_READ:          usize = 2;
pub const SLOT_YIELD:         usize = 3;
pub const SLOT_OPEN:          usize = 4;
pub const SLOT_CLOSE:         usize = 5;
pub const SLOT_SEEK:          usize = 6;
pub const SLOT_SPAWN:         usize = 7;
pub const SLOT_LIST:          usize = 8;
pub const SLOT_WAIT:          usize = 9;
pub const SLOT_PIPE:          usize = 10;
pub const SLOT_DUP:           usize = 11;
pub const SLOT_DUP2:          usize = 12;
pub const SLOT_MMAP:          usize = 13;
pub const SLOT_MUNMAP:        usize = 14;
pub const SLOT_FORK:          usize = 15;
pub const SLOT_EXEC:          usize = 16;
pub const SLOT_GETPID:        usize = 17;
pub const SLOT_GETPPID:       usize = 18;
pub const SLOT_CLOCK_GETTIME: usize = 19;
pub const SLOT_NANOSLEEP:     usize = 20;
pub const SLOT_SIGACTION:     usize = 21;
pub const SLOT_KILL:          usize = 22;
pub const SLOT_SIGRETURN:     usize = 23;
pub const SLOT_CREAT:         usize = 24;
pub const SLOT_UNLINK:        usize = 25;
pub const SLOT_FSTAT:         usize = 26;
pub const SLOT_SETFGPID:      usize = 27;
pub const SLOT_DISK_INFO:        usize = 28;
pub const SLOT_GETRANDOM:        usize = 29;
pub const SLOT_NICE:             usize = 30;
pub const SLOT_GETPRIORITY:      usize = 31;
pub const SLOT_SETPRIORITY:      usize = 32;
pub const SLOT_GETRLIMIT:        usize = 33;
pub const SLOT_SETRLIMIT:        usize = 34;
pub const SLOT_GETPGRP:          usize = 35;
pub const SLOT_SETPGID:          usize = 36;
pub const SLOT_GETSID:           usize = 37;
pub const SLOT_SETSID:           usize = 38;
pub const SLOT_TCGETPGRP:        usize = 39;
pub const SLOT_TCSETPGRP:        usize = 40;
pub const SLOT_UNAME:            usize = 41;
pub const SLOT_SYSINFO:          usize = 42;
pub const SLOT_SIGPROCMASK:      usize = 43;
pub const SLOT_SIGPENDING:       usize = 44;
pub const SLOT_SIGSUSPEND:       usize = 45;
pub const SLOT_GETRUSAGE:        usize = 46;
pub const SLOT_PRCTL:            usize = 47;
pub const SLOT_GETTIMEOFDAY:     usize = 48;
pub const SLOT_POLL:             usize = 49;
pub const SLOT_GETUID:           usize = 50;
pub const SLOT_GETGID:           usize = 51;
pub const SLOT_CHDIR:            usize = 52;
pub const SLOT_GETCWD:           usize = 53;
pub const SLOT_MKDIR:            usize = 54;
pub const SLOT_RMDIR:            usize = 55;
pub const SLOT_RENAME:           usize = 56;
pub const SLOT_GETDENTS64:       usize = 57;
pub const SLOT_TRUNCATE:         usize = 58;
pub const SLOT_FSYNC:            usize = 59;
pub const SLOT_IOCTL:            usize = 60;
pub const SLOT_TCGETATTR:        usize = 61;
pub const SLOT_TCSETATTR:        usize = 62;
pub const SLOT_FUTEX:            usize = 63;
pub const SLOT_EPOLL_CREATE1:    usize = 64;
pub const SLOT_EPOLL_CTL:        usize = 65;
pub const SLOT_EPOLL_WAIT:       usize = 66;
pub const SLOT_CLONE:            usize = 67;
pub const SLOT_SET_TLS:          usize = 68;
pub const SLOT_GETTID:           usize = 69;
// Display
pub const SLOT_FRAMEBUFFER_MAP:  usize = 70;
pub const SLOT_MKFIFO:           usize = 71;
pub const SLOT_SEM_OPEN:         usize = 72;
pub const SLOT_SEM_CLOSE:        usize = 73;
pub const SLOT_SEM_WAIT:         usize = 74;
pub const SLOT_SEM_TRYWAIT:      usize = 75;
pub const SLOT_SEM_POST:         usize = 76;
pub const SLOT_SEM_UNLINK:       usize = 77;
pub const SLOT_SEM_GETVALUE:     usize = 78;
pub const SLOT_SOCKET:           usize = 79;
pub const SLOT_BIND:             usize = 80;
pub const SLOT_LISTEN:           usize = 81;
pub const SLOT_ACCEPT:           usize = 82;
pub const SLOT_CONNECT:          usize = 83;
pub const SLOT_SEND:             usize = 84;
pub const SLOT_RECV:             usize = 85;
pub const SLOT_SHUTDOWN:         usize = 86;
pub const SLOT_GETSOCKNAME:      usize = 87;
pub const SLOT_GETPEERNAME:      usize = 88;
pub const SLOT_SOCKETPAIR:       usize = 89;
pub const SLOT_MQ_OPEN:          usize = 90;
pub const SLOT_MQ_CLOSE:         usize = 91;
pub const SLOT_MQ_SEND:          usize = 92;
pub const SLOT_MQ_RECEIVE:       usize = 93;
pub const SLOT_MQ_UNLINK:        usize = 94;
pub const SLOT_MQ_GETATTR:       usize = 95;
pub const SLOT_SELECT:           usize = 96;
pub const SLOT_UMASK:            usize = 97;
pub const SLOT_SIGALTSTACK:      usize = 98;
pub const SLOT_ALARM:            usize = 99;
pub const SLOT_MACHINE_REBOOT:   usize = 100;
pub const SLOT_MACHINE_POWEROFF: usize = 101;
pub const SLOT_GETEUID:          usize = 102;
pub const SLOT_GETEGID:          usize = 103;
pub const SLOT_SETUID:           usize = 104;
pub const SLOT_SETGID:           usize = 105;
pub const SLOT_CHMOD:            usize = 108;
pub const SLOT_MOUNT:            usize = 113;
pub const SLOT_GETMOUNTS:        usize = 114;
pub const SLOT_SYMLINK:          usize = 140;
pub const SLOT_GETGROUPS:        usize = 153;
pub const SLOT_SETREUID:         usize = 164;
pub const SLOT_SETREGID:         usize = 165;
pub const SLOT_SETGROUPS:        usize = 166;
