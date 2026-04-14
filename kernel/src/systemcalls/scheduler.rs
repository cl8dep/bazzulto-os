// scheduler.rs — Scheduler-related syscall implementations.
//
// Syscalls: nice, getpriority, setpriority, getrlimit, setrlimit,
//           getpgrp, setpgid, getsid, setsid, tcgetpgrp, tcsetpgrp
// Public:   terminal_foreground_pgid

use super::*;

/// Kernel-global foreground process group ID for the terminal.
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

// ---------------------------------------------------------------------------
// Phase 7 — Scheduler: nice, rlimits, process groups, sessions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_nice — add increment to the caller's nice value
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_nice(increment: i32) -> i64 {
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

/// sys_getpriority(which, who) — get process scheduling priority (nice value).
///
/// which=0 (PRIO_PROCESS), who=0 means calling process.
/// Other which/who values are accepted but ignored (no process group support yet).
///
/// Reference: POSIX.1-2017 getpriority(2).
pub(super) unsafe fn sys_getpriority(_which: i32, _who: i32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.nice as i64)
            .unwrap_or(EINVAL)
    })
}

/// sys_setpriority(which, who, prio) — set process scheduling priority (nice value).
///
/// which=0 (PRIO_PROCESS), who=0 means calling process.
/// prio is the new nice value in the range [NICE_MIN, NICE_MAX].
///
/// Reference: POSIX.1-2017 setpriority(2).
pub(super) unsafe fn sys_setpriority(_which: i32, _who: i32, prio: i32) -> i64 {
    if prio < crate::process::NICE_MIN as i32 || prio > crate::process::NICE_MAX as i32 {
        return EINVAL;
    }
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.nice = prio as i8;
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

pub(super) unsafe fn sys_getrlimit(resource: u32, rlim_ptr: *mut u64) -> i64 {
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

/// sys_setrlimit(resource, rlim_ptr) — set resource limit for the calling process.
///
/// `rlim_ptr` points to a struct rlimit: `{rlim_cur: u64, rlim_max: u64}`.
/// We apply only the soft limit (rlim_cur); the hard limit is accepted but ignored.
///
/// Reference: POSIX.1-2017 setrlimit(2), Linux include/uapi/sys/resource.h.
pub(super) unsafe fn sys_setrlimit(resource: u32, rlim_ptr: *const u64) -> i64 {
    if rlim_ptr.is_null()
        || !validate_user_pointer(rlim_ptr as u64, 2 * core::mem::size_of::<u64>())
    {
        return EINVAL;
    }
    // Read soft limit (rlim_cur) at offset 0; hard limit (rlim_max) at offset 8.
    let rlim_cur = *rlim_ptr;
    let _rlim_max = *rlim_ptr.add(1); // hard limit — accepted but ignored for now

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            match resource {
                crate::process::RLIMIT_NOFILE => {
                    process.resource_limits.open_files = rlim_cur;
                    0
                }
                crate::process::RLIMIT_AS => {
                    process.resource_limits.address_space_bytes = rlim_cur;
                    0
                }
                crate::process::RLIMIT_STACK => {
                    process.resource_limits.stack_bytes = rlim_cur;
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

pub(super) unsafe fn sys_getpgrp() -> i64 {
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
pub(super) unsafe fn sys_setpgid(pid: i32, pgid: i32) -> i64 {
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

pub(super) unsafe fn sys_getsid(pid: i32) -> i64 {
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
pub(super) unsafe fn sys_setsid() -> i64 {
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

pub(super) unsafe fn sys_tcgetpgrp(_fd: i32) -> i64 {
    TERMINAL_FOREGROUND_PGID.load(core::sync::atomic::Ordering::Relaxed) as i64
}

pub(super) unsafe fn sys_tcsetpgrp(_fd: i32, pgid: i32) -> i64 {
    if pgid <= 0 {
        return EINVAL;
    }
    TERMINAL_FOREGROUND_PGID.store(pgid as u32, core::sync::atomic::Ordering::Relaxed);
    0
}

// ---------------------------------------------------------------------------
// Phase 8 — POSIX syscalls: uname, sysinfo, sigprocmask, sigpending,
//           sigsuspend, getrusage, prctl, gettimeofday, poll
