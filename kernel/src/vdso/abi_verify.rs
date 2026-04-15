//! Compile-time verification: syscall numbers match their ABI-frozen values.
//!
//! The vDSO generates `svc #N` for slot N, and the kernel dispatches syscall
//! number N from the SVC immediate.  If someone renumbers a syscall constant
//! in `numbers`, the vDSO page (baked at compile time) will invoke the wrong
//! handler.  These assertions catch that at build time.
//!
//! Reference: docs/wiki/System-Calls.md (authoritative ABI table).

use crate::systemcalls::numbers;
use super::{SVC_BASE, VDSO_SLOT_COUNT};

/// Compile-time assert: the syscall constant has its documented ABI value.
macro_rules! abi_frozen {
    ($name:ident == $expected:literal) => {
        const _: () = {
            if numbers::$name != $expected {
                panic!(concat!(
                    "ABI VIOLATION: syscall numbers::",
                    stringify!($name),
                    " was renumbered — this breaks the frozen ABI (v0.2+).",
                ));
            }
        };
    };
}

// Core — process lifecycle and memory
abi_frozen!(EXIT          == 0);
abi_frozen!(WRITE         == 1);
abi_frozen!(READ          == 2);
abi_frozen!(YIELD         == 3);
abi_frozen!(OPEN          == 4);
abi_frozen!(CLOSE         == 5);
abi_frozen!(SEEK          == 6);
abi_frozen!(SPAWN         == 7);
abi_frozen!(LIST          == 8);
abi_frozen!(WAIT          == 9);
abi_frozen!(PIPE          == 10);
abi_frozen!(DUP           == 11);
abi_frozen!(DUP2          == 12);
abi_frozen!(MMAP          == 13);
abi_frozen!(MUNMAP        == 14);
abi_frozen!(FORK          == 15);
abi_frozen!(EXEC          == 16);
abi_frozen!(GETPID        == 17);
abi_frozen!(GETPPID       == 18);
abi_frozen!(CLOCK_GETTIME == 19);
abi_frozen!(NANOSLEEP     == 20);
abi_frozen!(SIGACTION     == 21);
abi_frozen!(KILL          == 22);
abi_frozen!(SIGRETURN     == 23);
abi_frozen!(CREAT         == 24);
abi_frozen!(UNLINK        == 25);
abi_frozen!(FSTAT         == 26);
abi_frozen!(SETFGPID      == 27);
abi_frozen!(DISK_INFO     == 28);

// System, scheduler, signals
abi_frozen!(GETRANDOM     == 29);
abi_frozen!(NICE          == 30);
abi_frozen!(GETPRIORITY   == 31);
abi_frozen!(SETPRIORITY   == 32);
abi_frozen!(GETRLIMIT     == 33);
abi_frozen!(SETRLIMIT     == 34);
abi_frozen!(GETPGRP       == 35);
abi_frozen!(SETPGID       == 36);
abi_frozen!(GETSID        == 37);
abi_frozen!(SETSID        == 38);
abi_frozen!(TCGETPGRP     == 39);
abi_frozen!(TCSETPGRP     == 40);
abi_frozen!(UNAME         == 41);
abi_frozen!(SYSINFO       == 42);
abi_frozen!(SIGPROCMASK   == 43);
abi_frozen!(SIGPENDING    == 44);
abi_frozen!(SIGSUSPEND    == 45);
abi_frozen!(GETRUSAGE     == 46);
abi_frozen!(PRCTL         == 47);
abi_frozen!(GETTIMEOFDAY  == 48);
abi_frozen!(POLL          == 49);
abi_frozen!(GETUID        == 50);
abi_frozen!(GETGID        == 51);

// VFS
abi_frozen!(CHDIR         == 52);
abi_frozen!(GETCWD        == 53);
abi_frozen!(MKDIR         == 54);
abi_frozen!(RMDIR         == 55);
abi_frozen!(RENAME        == 56);
abi_frozen!(GETDENTS64    == 57);
abi_frozen!(TRUNCATE      == 58);
abi_frozen!(FSYNC         == 59);

// Terminal / TTY
abi_frozen!(IOCTL         == 60);
abi_frozen!(TCGETATTR     == 61);
abi_frozen!(TCSETATTR     == 62);

// Threading
abi_frozen!(FUTEX         == 63);
abi_frozen!(EPOLL_CREATE1 == 64);
abi_frozen!(EPOLL_CTL     == 65);
abi_frozen!(EPOLL_WAIT    == 66);
abi_frozen!(CLONE         == 67);
abi_frozen!(SET_TLS       == 68);
abi_frozen!(GETTID        == 69);
abi_frozen!(FRAMEBUFFER_MAP == 70);

// FIFOs, semaphores, sockets, mqueues
abi_frozen!(MKFIFO        == 71);
abi_frozen!(SEM_OPEN      == 72);
abi_frozen!(SEM_CLOSE     == 73);
abi_frozen!(SEM_WAIT      == 74);
abi_frozen!(SEM_TRYWAIT   == 75);
abi_frozen!(SEM_POST      == 76);
abi_frozen!(SEM_UNLINK    == 77);
abi_frozen!(SEM_GETVALUE  == 78);
abi_frozen!(SOCKET        == 79);
abi_frozen!(BIND          == 80);
abi_frozen!(LISTEN        == 81);
abi_frozen!(ACCEPT        == 82);
abi_frozen!(CONNECT       == 83);
abi_frozen!(SEND          == 84);
abi_frozen!(RECV          == 85);
abi_frozen!(SHUTDOWN      == 86);
abi_frozen!(GETSOCKNAME   == 87);
abi_frozen!(GETPEERNAME   == 88);
abi_frozen!(SOCKETPAIR    == 89);
abi_frozen!(MQ_OPEN       == 90);
abi_frozen!(MQ_CLOSE      == 91);
abi_frozen!(MQ_SEND       == 92);
abi_frozen!(MQ_RECEIVE    == 93);
abi_frozen!(MQ_UNLINK     == 94);
abi_frozen!(MQ_GETATTR    == 95);
abi_frozen!(SELECT        == 96);
abi_frozen!(UMASK         == 97);
abi_frozen!(SIGALTSTACK   == 98);
abi_frozen!(ALARM         == 99);
abi_frozen!(MACHINE_REBOOT   == 100);
abi_frozen!(MACHINE_POWEROFF == 101);

// Identity (UID/GID)
abi_frozen!(GETEUID       == 102);
abi_frozen!(GETEGID       == 103);
abi_frozen!(SETUID        == 104);
abi_frozen!(SETGID        == 105);
abi_frozen!(SETEUID       == 106);
abi_frozen!(SETEGID       == 107);
abi_frozen!(CHMOD         == 108);
abi_frozen!(FCHMOD        == 109);
abi_frozen!(CHOWN         == 110);
abi_frozen!(FCHOWN        == 111);
abi_frozen!(LCHOWN        == 112);
abi_frozen!(MOUNT         == 113);
abi_frozen!(GETMOUNTS     == 114);

// musl/Linux ABI compatibility (115–161)
abi_frozen!(SET_TID_ADDRESS == 115);
abi_frozen!(SET_ROBUST_LIST == 116);
abi_frozen!(GET_ROBUST_LIST == 117);
abi_frozen!(EXIT_GROUP      == 118);
abi_frozen!(BRK             == 119);
abi_frozen!(OPENAT          == 120);
abi_frozen!(FSTATAT         == 121);
abi_frozen!(UNLINKAT        == 122);
abi_frozen!(MKDIRAT         == 123);
abi_frozen!(FTRUNCATE       == 124);
abi_frozen!(FDATASYNC       == 125);
abi_frozen!(PIPE2           == 126);
abi_frozen!(DUP3            == 127);
abi_frozen!(FCNTL           == 128);
abi_frozen!(MPROTECT        == 129);
abi_frozen!(ACCESS          == 130);
abi_frozen!(READLINK        == 131);
abi_frozen!(CLOCK_NANOSLEEP == 132);
abi_frozen!(CLOCK_GETRES    == 133);
abi_frozen!(WAITID          == 134);
abi_frozen!(TKILL           == 135);
abi_frozen!(TGKILL          == 136);
abi_frozen!(MREMAP          == 137);
abi_frozen!(MADVISE         == 138);
abi_frozen!(MSYNC           == 139);
abi_frozen!(SYMLINK         == 140);
abi_frozen!(LINK            == 141);
abi_frozen!(READLINKAT      == 142);
abi_frozen!(FCHOWNAT        == 143);
abi_frozen!(FCHMODAT        == 144);
abi_frozen!(FCHDIR          == 145);
abi_frozen!(STATX           == 146);
abi_frozen!(READV           == 147);
abi_frozen!(WRITEV          == 148);
abi_frozen!(PREAD64         == 149);
abi_frozen!(PWRITE64        == 150);
abi_frozen!(RENAMEAT        == 151);
abi_frozen!(TIMES           == 152);
abi_frozen!(GETGROUPS       == 153);
abi_frozen!(GETPGID         == 154);
abi_frozen!(CLOCK_SETTIME   == 155);
abi_frozen!(TIMER_CREATE    == 156);
abi_frozen!(TIMER_SETTIME   == 157);
abi_frozen!(TIMER_GETTIME   == 158);
abi_frozen!(TIMER_DELETE    == 159);
abi_frozen!(SETITIMER       == 160);
abi_frozen!(GETITIMER       == 161);

// SCM_RIGHTS support (added in v0.3)
abi_frozen!(SENDMSG         == 162);
abi_frozen!(RECVMSG         == 163);

// Verify the SVC encoding formula produces the correct instruction.
const _: () = {
    let expected_svc_0 = 0xD4000001_u32; // svc #0
    let generated = SVC_BASE | (0_u32 << 5);
    if generated != expected_svc_0 {
        panic!("SVC encoding formula is wrong for slot 0");
    }
};

// Verify slot count covers the highest vDSO-mapped syscall (GETMOUNTS = 114).
const _: () = {
    if VDSO_SLOT_COUNT <= 114 {
        panic!("VDSO_SLOT_COUNT must be > 114 (GETMOUNTS)");
    }
};
