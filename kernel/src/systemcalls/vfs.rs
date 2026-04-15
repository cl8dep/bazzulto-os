// vfs.rs — VFS-related syscall implementations.
//
// Syscalls: chdir, getcwd, umask, mkdir, rmdir, rename, getdents64,
//           truncate_fd, fsync, mkfifo, mount, getmounts
// Helpers:  copy_user_path, normalize_cwd_path, resolve_to_absolute

use super::*;

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
pub(super) unsafe fn copy_user_path<'a>(
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

pub(super) unsafe fn sys_chdir(path_ptr: *const u8) -> i64 {
    let mut buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path = match core::str::from_utf8(&buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
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
pub(super) fn normalize_cwd_path(path: &str) -> alloc::string::String {
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
pub(super) fn resolve_to_absolute(path: &str) -> alloc::string::String {
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

pub(super) unsafe fn sys_getcwd(buf_ptr: *mut u8, buf_len: usize) -> i64 {
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

pub(super) unsafe fn sys_umask(mask: u32) -> i64 {
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

pub(super) unsafe fn sys_mkdir(path_ptr: *const u8, mode: u32) -> i64 {
    let mut buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path_raw = match core::str::from_utf8(&buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let path_abs = resolve_to_absolute(path_raw);
    let path = path_abs.as_str();

    let (parent, dir_name) = match crate::fs::vfs_resolve_parent(path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    // DAC: POSIX mkdir(2) requires write+execute on the parent directory.
    {
        let denied = crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => crate::fs::vfs_check_access(
                    &parent.stat(), p.euid, p.egid,
                    &p.supplemental_groups, p.ngroups,
                    crate::fs::ACCESS_WRITE | crate::fs::ACCESS_EXECUTE,
                ).is_err(),
                None => false,
            }
        });
        if denied { return EACCES; }
    }

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

pub(super) unsafe fn sys_rmdir(path_ptr: *const u8) -> i64 {
    let mut buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path = match core::str::from_utf8(&buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let (parent, dir_name) = match crate::fs::vfs_resolve_parent(path) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };

    // DAC: POSIX rmdir(2) requires write+execute on the parent directory.
    {
        let denied = crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => crate::fs::vfs_check_access(
                    &parent.stat(), p.euid, p.egid,
                    &p.supplemental_groups, p.ngroups,
                    crate::fs::ACCESS_WRITE | crate::fs::ACCESS_EXECUTE,
                ).is_err(),
                None => false,
            }
        });
        if denied { return EACCES; }
    }

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

pub(super) unsafe fn sys_rename(
    old_ptr: *const u8,
    new_ptr: *const u8,
) -> i64 {
    let mut old_buf = [0u8; 512];
    let mut new_buf = [0u8; 512];
    let old_len = match copy_user_cstr(old_ptr, &mut old_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let old_path = match core::str::from_utf8(&old_buf[..old_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let new_len = match copy_user_cstr(new_ptr, &mut new_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let new_path = match core::str::from_utf8(&new_buf[..new_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
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

pub(super) unsafe fn sys_getdents64(fd: i32, buf_ptr: *mut u8, buf_len: usize) -> i64 {
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
// sys_truncate — truncate a file by path (Linux truncate(2) ABI)
//
// Linux syscall 45 / Bazzulto syscall TRUNCATE:
//   arg0 = path (NUL-terminated C string pointer)
//   arg1 = new length (u64)
//
// Reference: POSIX.1-2017 truncate(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_truncate(path_ptr: *const u8, length: u64) -> i64 {
    let mut buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let path = match core::str::from_utf8(&buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let abs_path = resolve_to_absolute(path);
    let inode = match crate::fs::vfs_resolve(&abs_path, None) {
        Ok(i) => i,
        Err(e) => return e.to_errno(),
    };
    match inode.truncate(length) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_truncate_fd — truncate an open file by file descriptor (ftruncate ABI)
//
// Called by posix_abi::sys_ftruncate (syscall FTRUNCATE).
// Reference: POSIX.1-2017 ftruncate(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_truncate_fd(fd: i32, new_size: u64) -> i64 {
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

pub(super) unsafe fn sys_fsync(fd: i32) -> i64 {
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

/// `mkfifo(path_ptr, mode) → i64`
///
/// Creates a FIFO inode at the given absolute path in the VFS.
/// Subsequent `open()` calls on the same path will share the same ring buffer.
/// The `mode` argument is accepted per Linux mkfifo(2) ABI but currently ignored
/// (FifoInode does not store a mode field).
///
/// Returns 0 on success, negative errno on error.
///
/// # Safety
/// `path_ptr` must be a valid user-space NUL-terminated string pointer.
pub(super) unsafe fn sys_mkfifo(path_ptr: *const u8, _mode: u32) -> i64 {
    let mut buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    if path_len == 0 {
        return EINVAL;
    }
    let path = match core::str::from_utf8(&buf[..path_len]) {
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

/// sys_mount — mount a filesystem at a VFS path.
///
/// Linux ABI (AArch64):
///   arg0: source_ptr   — NUL-terminated source path (device path or Bazzulto path)
///   arg1: target_ptr   — NUL-terminated mountpoint path
///   arg2: fstype_ptr   — NUL-terminated filesystem type string
///   arg3: mountflags   — mount flags (currently ignored)
///   arg4: data         — fs-specific data pointer (currently ignored)
///
/// Reference: Linux mount(2), POSIX.1-2017 mount(3p).
pub(super) unsafe fn sys_mount(
    source_ptr: u64,
    target_ptr: u64,
    fstype_ptr: u64,
    _mountflags: u64,
    _data: u64,
) -> i64 {
    const EPERM:   i64 = -1;
    const ENODEV:  i64 = -19;
    const EINVAL:  i64 = -22;
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

    // --- Read user strings via copy_user_cstr ---
    let mut source_buf = [0u8; 257];
    let source_len = {
        // copy_user_cstr takes [u8; 512]; use a local buf of the right size.
        let mut tmp = [0u8; 512];
        match copy_user_cstr(source_ptr as *const u8, &mut tmp) {
            Some(l) => { source_buf[..l].copy_from_slice(&tmp[..l]); l }
            None => return EINVAL,
        }
    };
    let mut target_buf = [0u8; 257];
    let target_len = {
        let mut tmp = [0u8; 512];
        match copy_user_cstr(target_ptr as *const u8, &mut tmp) {
            Some(l) => { target_buf[..l].copy_from_slice(&tmp[..l]); l }
            None => return EINVAL,
        }
    };
    let mut fstype_buf = [0u8; 17];
    let fstype_len = {
        let mut tmp = [0u8; 512];
        match copy_user_cstr(fstype_ptr as *const u8, &mut tmp) {
            Some(l) => {
                let copy_len = l.min(16);
                fstype_buf[..copy_len].copy_from_slice(&tmp[..copy_len]);
                copy_len
            }
            None => return EINVAL,
        }
    };

    if source_len == 0 || source_len > 256 { return EINVAL; }
    if target_len == 0 || target_len > 256 { return EINVAL; }
    if fstype_len == 0 { return EINVAL; }

    let source = match core::str::from_utf8(&source_buf[..source_len]) { Ok(s) => s, Err(_) => return EINVAL };
    let target = match core::str::from_utf8(&target_buf[..target_len]) { Ok(s) => s, Err(_) => return EINVAL };
    let fstype = match core::str::from_utf8(&fstype_buf[..fstype_len]) { Ok(s) => s, Err(_) => return EINVAL };

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
pub(super) unsafe fn sys_getmounts(buf_ptr: *mut u8, buf_len: usize) -> i64 {
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
