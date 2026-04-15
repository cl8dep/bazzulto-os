// identity.rs — POSIX UID/GID and file ownership syscall implementations.
//
// Syscalls: getuid, getgid, geteuid, getegid, setuid, setgid, seteuid,
//           setegid, setreuid, setregid, getgroups, setgroups,
//           chmod, fchmod, chown, fchown
//
// Reference: POSIX.1-2017 §4.12 (Process Identity), setuid(2), setreuid(2),
//            chmod(2), chown(2).

use super::*;

// ---------------------------------------------------------------------------
// UID/GID queries
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_getuid() -> i64 {
    crate::scheduler::with_scheduler(|s| {
        s.current_process().map(|p| p.uid as i64).unwrap_or(0)
    })
}

pub(super) unsafe fn sys_getgid() -> i64 {
    crate::scheduler::with_scheduler(|s| {
        s.current_process().map(|p| p.gid as i64).unwrap_or(0)
    })
}

pub(super) unsafe fn sys_geteuid() -> i64 {
    crate::scheduler::with_scheduler(|s| {
        s.current_process().map(|p| p.euid as i64).unwrap_or(0)
    })
}

pub(super) unsafe fn sys_getegid() -> i64 {
    crate::scheduler::with_scheduler(|s| {
        s.current_process().map(|p| p.egid as i64).unwrap_or(0)
    })
}

// ---------------------------------------------------------------------------
// setuid / setgid — POSIX.1-2017 setuid(2), setgid(2)
//
// Privileged (euid==0):  set uid, euid, AND suid to the new value.
// Unprivileged:          may only set euid to uid or suid.  Real UID unchanged.
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_setuid(new_uid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        if p.euid == 0 {
            p.uid  = new_uid;
            p.euid = new_uid;
            p.suid = new_uid;
            0
        } else if new_uid == p.uid || new_uid == p.suid {
            p.euid = new_uid;
            0
        } else {
            EPERM
        }
    })
}

pub(super) unsafe fn sys_setgid(new_gid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        if p.euid == 0 {
            p.gid  = new_gid;
            p.egid = new_gid;
            p.sgid = new_gid;
            0
        } else if new_gid == p.gid || new_gid == p.sgid {
            p.egid = new_gid;
            0
        } else {
            EPERM
        }
    })
}

// ---------------------------------------------------------------------------
// seteuid / setegid — set effective UID/GID only
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_seteuid(new_euid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        if p.euid == 0 || new_euid == p.uid || new_euid == p.suid {
            p.euid = new_euid;
            0
        } else {
            EPERM
        }
    })
}

pub(super) unsafe fn sys_setegid(new_egid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        if p.euid == 0 || new_egid == p.gid || new_egid == p.sgid {
            p.egid = new_egid;
            0
        } else {
            EPERM
        }
    })
}

// ---------------------------------------------------------------------------
// setreuid / setregid — POSIX.1-2017 setreuid(2), setregid(2)
//
// -1 (u32::MAX) means "no change" for that argument.
//
// Unprivileged rules for setreuid:
//   - ruid: may be set to current uid or euid only
//   - euid: may be set to current uid, euid, or suid only
// If ruid is set OR euid is set to a value != old uid, the saved UID
// is set to the new effective UID.
//
// Privileged (euid==0): any values allowed.
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_setreuid(ruid: u32, euid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        let old_uid  = p.uid;
        let old_euid = p.euid;

        // Validate ruid.
        if ruid != u32::MAX && p.euid != 0 {
            if ruid != p.uid && ruid != p.euid {
                return EPERM;
            }
        }
        // Validate euid.
        if euid != u32::MAX && p.euid != 0 {
            if euid != p.uid && euid != p.euid && euid != p.suid {
                return EPERM;
            }
        }

        if ruid != u32::MAX { p.uid = ruid; }
        if euid != u32::MAX { p.euid = euid; }

        // POSIX: if ruid was set, or euid was set to something other than the
        // old real UID, set the saved UID to the new effective UID.
        if ruid != u32::MAX || (euid != u32::MAX && euid != old_uid) {
            p.suid = p.euid;
        }
        0
    })
}

pub(super) unsafe fn sys_setregid(rgid: u32, egid: u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };
        let old_gid  = p.gid;

        if rgid != u32::MAX && p.euid != 0 {
            if rgid != p.gid && rgid != p.egid {
                return EPERM;
            }
        }
        if egid != u32::MAX && p.euid != 0 {
            if egid != p.gid && egid != p.egid && egid != p.sgid {
                return EPERM;
            }
        }

        if rgid != u32::MAX { p.gid = rgid; }
        if egid != u32::MAX { p.egid = egid; }

        if rgid != u32::MAX || (egid != u32::MAX && egid != old_gid) {
            p.sgid = p.egid;
        }
        0
    })
}

// ---------------------------------------------------------------------------
// getgroups / setgroups — supplementary group IDs
//
// Reference: POSIX.1-2017 getgroups(2), setgroups(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_getgroups(size: i32, list_ptr: *mut u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process() { Some(p) => p, None => return EPERM };
        let ngroups = p.ngroups as i64;

        if size == 0 {
            // Return the number of supplementary groups.
            return ngroups;
        }
        if (size as usize) < p.ngroups {
            return EINVAL;
        }
        if list_ptr.is_null() {
            return EINVAL;
        }

        // Validate user pointer.
        if !crate::systemcalls::validate_user_pointer(
            list_ptr as u64, p.ngroups * core::mem::size_of::<u32>()
        ) {
            return EFAULT;
        }

        for i in 0..p.ngroups {
            core::ptr::write(list_ptr.add(i), p.supplemental_groups[i]);
        }
        ngroups
    })
}

pub(super) unsafe fn sys_setgroups(size: usize, list_ptr: *const u32) -> i64 {
    crate::scheduler::with_scheduler(|s| {
        let p = match s.current_process_mut() { Some(p) => p, None => return EPERM };

        // Only root can set supplementary groups.
        if p.euid != 0 {
            return EPERM;
        }
        if size > 16 {
            return EINVAL;
        }
        if size > 0 && list_ptr.is_null() {
            return EINVAL;
        }

        if size > 0 {
            if !crate::systemcalls::validate_user_pointer(
                list_ptr as u64, size * core::mem::size_of::<u32>()
            ) {
                return EFAULT;
            }
            for i in 0..size {
                p.supplemental_groups[i] = core::ptr::read(list_ptr.add(i));
            }
        }
        p.ngroups = size;
        0
    })
}

// ---------------------------------------------------------------------------
// chmod / fchmod — set file mode bits
//
// Permission: euid==0 or euid==inode.uid.
// Reference: POSIX.1-2017 chmod(2), fchmod(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_chmod(path_ptr: *const u8, mode: u32) -> i64 {
    let mut path_buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut path_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path = match core::str::from_utf8(&path_buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let (euid, cwd) = crate::scheduler::with_scheduler(|s| {
        let p = s.current_process().unwrap();
        (p.euid, p.cwd.clone())
    });

    let inode = match crate::fs::vfs_resolve(path, cwd.as_ref()) {
        Ok(i) => i,
        Err(e) => return e.to_errno(),
    };

    let stat = inode.stat();
    // Permission check: must be owner or root.
    if euid != 0 && euid != stat.uid {
        return EPERM;
    }

    // Apply only the permission bits (lower 12 bits: setuid+setgid+sticky+rwxrwxrwx).
    let new_mode = (stat.mode & !0o7777) | (mode as u64 & 0o7777);
    match inode.set_mode(new_mode) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

pub(super) unsafe fn sys_fchmod(fd: i32, mode: u32) -> i64 {
    let (euid, inode) = match get_inode_from_fd(fd) {
        Some(v) => v,
        None => return EBADF,
    };

    let stat = inode.stat();
    if euid != 0 && euid != stat.uid {
        return EPERM;
    }

    let new_mode = (stat.mode & !0o7777) | (mode as u64 & 0o7777);
    match inode.set_mode(new_mode) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// chown / fchown — change file owner/group
//
// Only root (euid==0) can change the owner UID.
// Owner or root can change the group to a group in their supplementary groups.
// u32::MAX (-1) means "no change" for that argument.
//
// Reference: POSIX.1-2017 chown(2), fchown(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_chown(path_ptr: *const u8, new_uid: u32, new_gid: u32) -> i64 {
    let mut path_buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut path_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path = match core::str::from_utf8(&path_buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let (euid, inode_uid, groups, ngroups, cwd) = crate::scheduler::with_scheduler(|s| {
        let p = s.current_process().unwrap();
        (p.euid, p.uid, p.supplemental_groups, p.ngroups, p.cwd.clone())
    });

    let inode = match crate::fs::vfs_resolve(path, cwd.as_ref()) {
        Ok(i) => i,
        Err(e) => return e.to_errno(),
    };

    chown_impl(&*inode, new_uid, new_gid, euid, inode_uid, &groups, ngroups)
}

pub(super) unsafe fn sys_fchown(fd: i32, new_uid: u32, new_gid: u32) -> i64 {
    let result = crate::scheduler::with_scheduler(|s| {
        let p = s.current_process()?;
        let euid = p.euid;
        let groups = p.supplemental_groups;
        let ngroups = p.ngroups;
        let fd_table = p.file_descriptor_table.lock();
        let descriptor = fd_table.get(fd as usize)?;
        match descriptor {
            crate::fs::FileDescriptor::InoFile { inode, .. } => {
                Some((euid, groups, ngroups, inode.clone()))
            }
            _ => None,
        }
    });

    let (euid, groups, ngroups, inode) = match result {
        Some(v) => v,
        None => return EBADF,
    };

    chown_impl(&*inode, new_uid, new_gid, euid, euid, &groups, ngroups)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Common chown implementation for sys_chown and sys_fchown.
fn chown_impl(
    inode: &dyn crate::fs::Inode,
    new_uid: u32,
    new_gid: u32,
    euid: u32,
    _caller_uid: u32,
    groups: &[u32; 16],
    ngroups: usize,
) -> i64 {
    let stat = inode.stat();

    // Changing UID: root only.
    if new_uid != u32::MAX && new_uid != stat.uid {
        if euid != 0 {
            return EPERM;
        }
    }

    // Changing GID: root, or owner if new_gid is in their groups.
    if new_gid != u32::MAX && new_gid != stat.gid {
        if euid != 0 {
            if euid != stat.uid {
                return EPERM;
            }
            // Check if new_gid is in the caller's supplementary groups.
            let mut found = false;
            for i in 0..ngroups {
                if groups[i] == new_gid { found = true; break; }
            }
            if !found {
                return EPERM;
            }
        }
    }

    let uid = if new_uid != u32::MAX { new_uid } else { stat.uid };
    let gid = if new_gid != u32::MAX { new_gid } else { stat.gid };

    match inode.set_owner(uid, gid) {
        Ok(()) => {
            // POSIX: chown clears setuid/setgid bits if the caller is not root.
            if euid != 0 {
                let cleared_mode = stat.mode & !0o6000; // clear SUID + SGID
                let _ = inode.set_mode(cleared_mode);
            }
            0
        }
        Err(e) => e.to_errno(),
    }
}

/// Get the inode and caller's euid from an fd.
unsafe fn get_inode_from_fd(fd: i32) -> Option<(u32, alloc::sync::Arc<dyn crate::fs::Inode>)> {
    crate::scheduler::with_scheduler(|s| {
        let p = s.current_process()?;
        let euid = p.euid;
        let fd_table = p.file_descriptor_table.lock();
        let descriptor = fd_table.get(fd as usize)?;
        match descriptor {
            crate::fs::FileDescriptor::InoFile { inode, .. } => {
                Some((euid, inode.clone()))
            }
            _ => None,
        }
    })
}
