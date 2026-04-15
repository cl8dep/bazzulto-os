// io.rs — I/O syscall implementations.
//
// Syscalls: write, read, yield, open, close, seek, list, pipe, dup, dup2,
//           creat, unlink, fstat

use super::*;

// ---------------------------------------------------------------------------
// sys_write
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_write(fd: i32, buffer_ptr: *const u8, length: usize) -> i64 {
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

pub(super) unsafe fn sys_read(fd: i32, buffer_ptr: *mut u8, length: usize) -> i64 {
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

pub(super) unsafe fn sys_yield() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.schedule();
    });
    0
}

// ---------------------------------------------------------------------------
// sys_open
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_open(name_ptr: *const u8, _name_length: usize, flags: i32, mode: u32) -> i64 {
    let mut name_buf = [0u8; 512];
    let name_len = match copy_user_cstr(name_ptr, &mut name_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let name_raw = match core::str::from_utf8(&name_buf[..name_len]) {
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
                    // DAC permission check (POSIX.1-2017 open(2)).
                    //
                    // Enforce POSIX file permissions before granting access.
                    // This check ensures that e.g. a uid=1000 process cannot
                    // read /system/config/shadow (mode 0600, owner root).
                    {
                        let access_needed = {
                            let mut a = 0u32;
                            let access_mode = flags & 0o3;
                            if access_mode == 0 || access_mode == 2 { a |= crate::fs::ACCESS_READ; }
                            if access_mode == 1 || access_mode == 2 { a |= crate::fs::ACCESS_WRITE; }
                            a
                        };
                        let denied = crate::scheduler::with_scheduler(|scheduler| {
                            match scheduler.current_process() {
                                Some(p) => crate::fs::vfs_check_access(
                                    &inode.stat(), p.euid, p.egid,
                                    &p.supplemental_groups, p.ngroups, access_needed,
                                ).is_err(),
                                None => false,
                            }
                        });
                        if denied { return EACCES; }
                    }

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
                    // DAC check (same as absolute path branch above).
                    {
                        let access_needed = {
                            let mut a = 0u32;
                            let access_mode = flags & 0o3;
                            if access_mode == 0 || access_mode == 2 { a |= crate::fs::ACCESS_READ; }
                            if access_mode == 1 || access_mode == 2 { a |= crate::fs::ACCESS_WRITE; }
                            a
                        };
                        let denied = crate::scheduler::with_scheduler(|scheduler| {
                            match scheduler.current_process() {
                                Some(p) => crate::fs::vfs_check_access(
                                    &inode.stat(), p.euid, p.egid,
                                    &p.supplemental_groups, p.ngroups, access_needed,
                                ).is_err(),
                                None => false,
                            }
                        });
                        if denied { return EACCES; }
                    }
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

pub(super) unsafe fn sys_list(buffer_ptr: *mut u8, buffer_length: usize) -> i64 {
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


pub(super) unsafe fn sys_pipe(fd_pair_ptr: *mut i32) -> i64 {
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

pub(super) unsafe fn sys_dup(source_fd: i32) -> i64 {
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

pub(super) unsafe fn sys_dup2(source_fd: i32, destination_fd: i32) -> i64 {
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
// sys_creat — create or truncate a file via VFS
// ---------------------------------------------------------------------------

/// creat(path, mode) — equivalent to open(path, O_CREAT|O_WRONLY|O_TRUNC, mode).
/// Always truncates. Linux ABI: arg0=path, arg1=mode. name_length removed.
pub(super) unsafe fn sys_creat(name_ptr: *const u8, _mode: u32) -> i64 {
    let mut name_buf = [0u8; 512];
    let name_len = match copy_user_cstr(name_ptr, &mut name_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let name_raw = match core::str::from_utf8(&name_buf[..name_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Resolve relative paths against cwd before VFS lookup.
    let name_abs = resolve_to_absolute(name_raw);
    let name = name_abs.as_str();

    // Resolve or create via VFS. Always truncate (Linux creat() semantics).
    let inode = match crate::fs::vfs_resolve(name, None) {
        Ok(existing) => {
            // File exists: always truncate per Linux creat(2) = O_CREAT|O_WRONLY|O_TRUNC.
            let _ = existing.truncate(0);
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

pub(super) unsafe fn sys_unlink(name_ptr: *const u8) -> i64 {
    let mut name_buf = [0u8; 512];
    let name_len = match copy_user_cstr(name_ptr, &mut name_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let name_raw = match core::str::from_utf8(&name_buf[..name_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let name_abs = resolve_to_absolute(name_raw);
    let name = name_abs.as_str();

    // Resolve to parent + name, then call unlink via VFS.
    // DAC: POSIX unlink(2) requires write+execute on the parent directory.
    match crate::fs::vfs_resolve_parent(name) {
        Ok((parent, file_name)) => {
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
            match parent.unlink(&file_name) {
                Ok(()) => 0,
                Err(error) => error.to_errno(),
            }
        }
        Err(error) => error.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_fstat — get file metadata by file descriptor (Linux ABI)
//
// Writes a Linux-compatible stat64 struct to stat_ptr.
// AArch64 Linux struct stat layout (key fields):
//   offset  0:  u64  st_dev
//   offset  8:  u64  st_ino
//   offset 16:  u32  st_mode
//   offset 20:  u32  st_nlink
//   offset 24:  u32  st_uid
//   offset 28:  u32  st_gid
//   offset 32:  u64  st_rdev
//   offset 40:  u64  st_size
//   offset 48:  u64  st_blksize
//   offset 56:  u64  st_blocks (512-byte units)
//   offset 64:  u64  st_atime_sec
//   offset 72:  u64  st_atime_nsec
//   offset 80:  u64  st_mtime_sec
//   offset 88:  u64  st_mtime_nsec
//   offset 96:  u64  st_ctime_sec
//   offset 104: u64  st_ctime_nsec
// Total: 128 bytes minimum.
//
// Reference: Linux include/uapi/asm-generic/stat.h.
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_fstat(fd: i32, stat_ptr: *mut u8) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    // Minimum size required: 128 bytes to hold all fields through st_ctime_nsec.
    const STAT_SIZE: usize = 128;
    if !validate_user_pointer(stat_ptr as u64, STAT_SIZE) {
        return EFAULT;
    }

    // Retrieve stat from the fd table entry.
    let inode_stat = crate::scheduler::with_scheduler(|scheduler| {
        let process = scheduler.current_process()?;
        let table = process.file_descriptor_table.lock();
        match table.get(fd as usize) {
            Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) => {
                Some((inode.stat(), inode.inode_type()))
            }
            Some(crate::fs::vfs::FileDescriptor::RamFsFile { data, .. }) => {
                Some((crate::fs::InodeStat {
                    inode_number: 0,
                    size: data.len() as u64,
                    // S_IFREG = 0o100000 | r--r--r-- = 0o444.
                    mode: 0o100444,
                    nlinks: 1,
                    uid: 0,
                    gid: 0,
                }, crate::fs::InodeType::RegularFile))
            }
            Some(crate::fs::vfs::FileDescriptor::Pipe(_)) => {
                Some((crate::fs::InodeStat {
                    inode_number: 0,
                    size: 0,
                    // S_IFIFO = 0o010000 (Linux/POSIX named pipe file type bit).
                    mode: 0o010000,
                    nlinks: 1,
                    uid: 0,
                    gid: 0,
                }, crate::fs::InodeType::Fifo))
            }
            Some(crate::fs::vfs::FileDescriptor::Tty) => {
                Some((crate::fs::InodeStat {
                    inode_number: 0,
                    size: 0,
                    // S_IFCHR = 0o020000 (character device) | rw-rw-rw- = 0o666.
                    mode: 0o020666,
                    nlinks: 1,
                    uid: 0,
                    gid: 0,
                }, crate::fs::InodeType::CharDevice))
            }
            _ => None,
        }
    });

    let (stat, _inode_type) = match inode_stat {
        Some(s) => s,
        None => return EBADF,
    };

    // Zero the entire stat buffer, then fill the fields we know.
    let out = core::slice::from_raw_parts_mut(stat_ptr, STAT_SIZE);
    out.fill(0);

    // st_ino at offset 8.
    out[8..16].copy_from_slice(&stat.inode_number.to_le_bytes());
    // st_mode at offset 16 (u32).
    out[16..20].copy_from_slice(&(stat.mode as u32).to_le_bytes());
    // st_nlink at offset 20 (u32).
    out[20..24].copy_from_slice(&(stat.nlinks as u32).to_le_bytes());
    // st_uid at offset 24 (u32).
    out[24..28].copy_from_slice(&stat.uid.to_le_bytes());
    // st_gid at offset 28 (u32).
    out[28..32].copy_from_slice(&stat.gid.to_le_bytes());
    // st_size at offset 40.
    out[40..48].copy_from_slice(&stat.size.to_le_bytes());
    // st_blksize at offset 48 (4096 bytes = one page).
    out[48..56].copy_from_slice(&4096u64.to_le_bytes());
    // st_blocks at offset 56 (512-byte block units).
    let blocks_512 = (stat.size + 511) / 512;
    out[56..64].copy_from_slice(&blocks_512.to_le_bytes());

    0
}

// ---------------------------------------------------------------------------
// sys_close — close a file descriptor
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_close(fd: i32) -> i64 {
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
// sys_seek — reposition read/write file offset
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_seek(fd: i32, offset: i64, whence: i32) -> i64 {
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
