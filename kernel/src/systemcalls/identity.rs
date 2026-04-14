// identity.rs — POSIX UID/GID syscall implementations.
//
// Syscalls: getuid, getgid, geteuid, getegid, setuid, setgid, seteuid, setegid,
//           chmod, fchmod, chown, fchown

use super::*;

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

pub(super) unsafe fn sys_getuid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.uid as i64)
            .unwrap_or(0)
    })
}

pub(super) unsafe fn sys_getgid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.gid as i64)
            .unwrap_or(0)
    })
}

pub(super) unsafe fn sys_geteuid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.euid as i64)
            .unwrap_or(0)
    })
}

pub(super) unsafe fn sys_getegid() -> i64 {
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
pub(super) unsafe fn sys_setuid(new_uid: u32) -> i64 {
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
pub(super) unsafe fn sys_setgid(new_gid: u32) -> i64 {
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
pub(super) unsafe fn sys_seteuid(new_euid: u32) -> i64 {
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
pub(super) unsafe fn sys_setegid(new_egid: u32) -> i64 {
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

pub(super) unsafe fn sys_chmod(_path_ptr: *const u8, _mode: u32) -> i64 {
    // TODO: update inode mode bits when InodeStat carries owner info.
    0
}

pub(super) unsafe fn sys_fchmod(_fd: i32, _mode: u32) -> i64 {
    // TODO: update inode mode bits.
    0
}

pub(super) unsafe fn sys_chown(_path_ptr: *const u8, _new_uid: u32, _new_gid: u32) -> i64 {
    // TODO: update inode owner when InodeStat carries owner info.
    0
}

pub(super) unsafe fn sys_fchown(_fd: i32, _new_uid: u32, _new_gid: u32) -> i64 {
    // TODO: update inode owner.
    0
}

// ---------------------------------------------------------------------------
