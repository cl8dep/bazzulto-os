// posix_abi.rs — musl/Linux ABI compatibility syscall stubs (numbers 115–161).
// These handlers provide the Linux-ABI interface that musl libc expects.

use super::*;

// ---------------------------------------------------------------------------
// sys_set_tid_address — store tidptr in thread struct, return current TID
// ---------------------------------------------------------------------------

pub unsafe fn sys_set_tid_address(tidptr: u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        let tid = scheduler.current_pid().index as i64;
        if let Some(process) = scheduler.current_process_mut() {
            process.clear_child_tid = tidptr;
        }
        tid
    })
}

// ---------------------------------------------------------------------------
// sys_set_robust_list — store the robust futex list head for the calling thread
// ---------------------------------------------------------------------------

pub unsafe fn sys_set_robust_list(head: u64, length: usize) -> i64 {
    // Reference: Linux robust_list(7) — kernel stores head ptr per-thread and
    // walks the list on thread death to unlock any futexes the thread held.
    if length != core::mem::size_of::<u64>() * 3 {
        return EINVAL;
    }
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.robust_list_head = head;
        }
        0i64
    })
}

// ---------------------------------------------------------------------------
// sys_get_robust_list — return the robust futex list head for a pid/tid
// ---------------------------------------------------------------------------

pub unsafe fn sys_get_robust_list(pid: i32, head_ptr: u64, len_ptr: u64) -> i64 {
    if head_ptr == 0 || len_ptr == 0 {
        return EINVAL;
    }
    crate::scheduler::with_scheduler(|scheduler| {
        let target_pid = if pid == 0 {
            scheduler.current_pid()
        } else {
            crate::process::Pid::new(pid as u16, 1)
        };
        let head = match scheduler.process(target_pid) {
            Some(p) => p.robust_list_head,
            None => return ESRCH,
        };
        if !validate_user_pointer(head_ptr, core::mem::size_of::<u64>()) {
            return EFAULT;
        }
        if !validate_user_pointer(len_ptr, core::mem::size_of::<u64>()) {
            return EFAULT;
        }
        *(head_ptr as *mut u64) = head;
        *(len_ptr as *mut u64) = core::mem::size_of::<u64>() as u64 * 3;
        0i64
    })
}

// ---------------------------------------------------------------------------
// sys_exit_group — terminate all threads in the process group (= exit for now)
// ---------------------------------------------------------------------------

pub unsafe fn sys_exit_group(exit_code: i32) -> i64 {
    // For a single-threaded process this is identical to sys_exit.
    // A multi-threaded implementation would signal all sibling threads first.
    super::process::sys_exit(exit_code)
}

// ---------------------------------------------------------------------------
// sys_brk — extend or query the program break (heap end)
// ---------------------------------------------------------------------------

pub unsafe fn sys_brk(new_brk: u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            if new_brk == 0 {
                // Query: return current brk.
                return process.brk_current as i64;
            }
            // Extend: must not shrink below brk_base.
            if new_brk < process.brk_base {
                return process.brk_current as i64;
            }
            process.brk_current = new_brk;
            // Register or grow the anonymous demand region for the heap.
            let heap_region_exists = process.mmap_regions.iter()
                .any(|r| r.base == process.brk_base);
            if !heap_region_exists {
                process.mmap_regions.push(crate::process::MmapRegion {
                    base:   process.brk_base,
                    length: new_brk - process.brk_base,
                    demand: true,
                    backing: crate::process::MmapBacking::Anonymous,
                });
            } else {
                for r in process.mmap_regions.iter_mut() {
                    if r.base == process.brk_base {
                        r.length = new_brk - process.brk_base;
                        break;
                    }
                }
            }
            new_brk as i64
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_openat — open relative to a directory file descriptor
// ---------------------------------------------------------------------------

pub unsafe fn sys_openat(dirfd: i32, path_ptr: *const u8, flags: i32, mode: u32) -> i64 {
    // AT_FDCWD (-100): use process cwd — delegate to sys_open with cwd resolution.
    // Other dirfd values would require resolving relative to that fd's path.
    // For now: only AT_FDCWD is supported; other dirfd values return EBADF.
    if dirfd != AT_FDCWD && dirfd >= 0 {
        // TODO: resolve relative to open directory fd.
        return EBADF;
    }
    // Delegate to the existing sys_open which already handles path resolution.
    super::io::sys_open(path_ptr, 0, flags, mode)
}

// ---------------------------------------------------------------------------
// sys_fstatat — stat relative to a directory file descriptor
// ---------------------------------------------------------------------------

pub unsafe fn sys_fstatat(dirfd: i32, path_ptr: *const u8, stat_ptr: *mut u64, flags: i32) -> i64 {
    // AT_FDCWD or absolute path: resolve path and fill stat buffer directly.
    // AT_EMPTY_PATH + dirfd: fstat-by-fd semantics (not yet implemented here).
    // sys_fstat is now fd-based, so we resolve the path to VFS here.
    let _ = (dirfd, flags);
    if path_ptr.is_null() {
        return EINVAL;
    }
    let mut buf = [0u8; 512];
    let len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let path = match core::str::from_utf8(&buf[..len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let abs_path = super::vfs::resolve_to_absolute(path);

    // Minimum stat buffer size: 128 bytes for all fields through ctime_nsec.
    const STAT_SIZE: usize = 128;
    if !validate_user_pointer(stat_ptr as u64, STAT_SIZE) {
        return EFAULT;
    }
    let out = core::slice::from_raw_parts_mut(stat_ptr as *mut u8, STAT_SIZE);

    // Try VFS first.
    if let Ok(inode) = crate::fs::vfs_resolve(&abs_path, None) {
        let stat = inode.stat();
        out.fill(0);
        out[8..16].copy_from_slice(&stat.inode_number.to_le_bytes());
        out[16..20].copy_from_slice(&(stat.mode as u32).to_le_bytes());
        out[20..24].copy_from_slice(&(stat.nlinks as u32).to_le_bytes());
        out[40..48].copy_from_slice(&stat.size.to_le_bytes());
        out[48..56].copy_from_slice(&4096u64.to_le_bytes());
        let blocks = (stat.size + 511) / 512;
        out[56..64].copy_from_slice(&blocks.to_le_bytes());
        return 0;
    }

    // Fall back to ramfs.
    if let Some(data) = crate::fs::ramfs_find(&abs_path) {
        out.fill(0);
        // S_IFREG | r--r--r-- = 0o100444.
        out[16..20].copy_from_slice(&0o100444u32.to_le_bytes());
        out[20..24].copy_from_slice(&1u32.to_le_bytes());
        out[40..48].copy_from_slice(&(data.len() as u64).to_le_bytes());
        out[48..56].copy_from_slice(&4096u64.to_le_bytes());
        return 0;
    }

    ENOENT
}

// ---------------------------------------------------------------------------
// sys_unlinkat — unlink relative to a directory file descriptor
// ---------------------------------------------------------------------------

pub unsafe fn sys_unlinkat(dirfd: i32, path_ptr: *const u8, flags: i32) -> i64 {
    let _ = dirfd;
    if path_ptr.is_null() {
        return EINVAL;
    }
    if flags & AT_REMOVEDIR != 0 {
        // rmdir semantics: sys_rmdir now takes only a NUL-terminated path pointer.
        super::vfs::sys_rmdir(path_ptr)
    } else {
        // unlink semantics: sys_unlink now takes only a NUL-terminated path pointer.
        super::io::sys_unlink(path_ptr)
    }
}

// ---------------------------------------------------------------------------
// sys_mkdirat — mkdir relative to a directory file descriptor
// ---------------------------------------------------------------------------

pub unsafe fn sys_mkdirat(dirfd: i32, path_ptr: *const u8, mode: u32) -> i64 {
    let _ = dirfd;
    if path_ptr.is_null() {
        return EINVAL;
    }
    // sys_mkdir now takes only a NUL-terminated path pointer and mode.
    super::vfs::sys_mkdir(path_ptr, mode)
}

// ---------------------------------------------------------------------------
// sys_ftruncate — truncate an open file by fd
// ---------------------------------------------------------------------------

pub unsafe fn sys_ftruncate(fd: i32, length: u64) -> i64 {
    // Delegate to the existing truncate-by-fd implementation.
    super::vfs::sys_truncate_fd(fd, length)
}

// ---------------------------------------------------------------------------
// sys_fdatasync — flush file data (alias to fsync for now)
// ---------------------------------------------------------------------------

pub unsafe fn sys_fdatasync(fd: i32) -> i64 {
    super::vfs::sys_fsync(fd)
}

// ---------------------------------------------------------------------------
// sys_pipe2 — create a pipe with flags (O_CLOEXEC, O_NONBLOCK)
// ---------------------------------------------------------------------------

pub unsafe fn sys_pipe2(fd_pair_ptr: *mut i32, flags: i32) -> i64 {
    let result = super::io::sys_pipe(fd_pair_ptr);
    if result != 0 {
        return result;
    }
    // Apply O_CLOEXEC and O_NONBLOCK flags to both fds.
    if flags & (O_CLOEXEC | O_NONBLOCK) != 0 {
        let read_fd  = *fd_pair_ptr;
        let write_fd = *fd_pair_ptr.add(1);
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        if let Some(arc) = fd_table_arc {
            let mut guard = arc.lock();
            for fd in [read_fd as usize, write_fd as usize] {
                if flags & O_CLOEXEC != 0 {
                    crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.cloexec_mask, fd);
                }
                if flags & O_NONBLOCK != 0 {
                    crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.nonblock_mask, fd);
                }
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sys_dup3 — dup2 with O_CLOEXEC support
// ---------------------------------------------------------------------------

pub unsafe fn sys_dup3(old_fd: i32, new_fd: i32, flags: i32) -> i64 {
    let result = super::io::sys_dup2(old_fd, new_fd);
    if result < 0 {
        return result;
    }
    if flags & O_CLOEXEC != 0 {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        if let Some(arc) = fd_table_arc {
            let mut guard = arc.lock();
            crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.cloexec_mask, new_fd as usize);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// sys_fcntl — file control operations
// ---------------------------------------------------------------------------

/// F_GETFD — get file descriptor flags (FD_CLOEXEC).
const F_GETFD: i32 = 1;
/// F_SETFD — set file descriptor flags (FD_CLOEXEC).
const F_SETFD: i32 = 2;
/// F_GETFL — get file status flags (O_NONBLOCK etc.).
const F_GETFL: i32 = 3;
/// F_SETFL — set file status flags.
const F_SETFL: i32 = 4;
/// F_DUPFD — duplicate fd, allocating the lowest fd >= arg.
const F_DUPFD: i32 = 0;
/// F_DUPFD_CLOEXEC — like F_DUPFD but also sets FD_CLOEXEC.
const F_DUPFD_CLOEXEC: i32 = 1030;
/// FD_CLOEXEC flag bit.
const FD_CLOEXEC: i32 = 1;

pub unsafe fn sys_fcntl(fd: i32, cmd: i32, arg: u64) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EBADF,
    };
    let mut guard = fd_table_arc.lock();
    match cmd {
        F_GETFD => {
            let cloexec = crate::fs::vfs::FileDescriptorTable::mask_test(&guard.cloexec_mask, fd as usize);
            if cloexec { FD_CLOEXEC as i64 } else { 0 }
        }
        F_SETFD => {
            if arg as i32 & FD_CLOEXEC != 0 {
                crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.cloexec_mask, fd as usize);
            } else {
                crate::fs::vfs::FileDescriptorTable::mask_clear(&mut guard.cloexec_mask, fd as usize);
            }
            0
        }
        F_GETFL => {
            let nonblock = crate::fs::vfs::FileDescriptorTable::mask_test(&guard.nonblock_mask, fd as usize);
            if nonblock { O_NONBLOCK as i64 } else { 0 }
        }
        F_SETFL => {
            if arg as i32 & O_NONBLOCK != 0 {
                crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.nonblock_mask, fd as usize);
            } else {
                crate::fs::vfs::FileDescriptorTable::mask_clear(&mut guard.nonblock_mask, fd as usize);
            }
            0
        }
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let min_fd = arg as usize;
            let new_fd = guard.dup_at_or_above(fd as usize, min_fd);
            if new_fd < 0 {
                return EBADF;
            }
            if cmd == F_DUPFD_CLOEXEC {
                crate::fs::vfs::FileDescriptorTable::mask_set(&mut guard.cloexec_mask, new_fd as usize);
            }
            new_fd as i64
        }
        _ => EINVAL,
    }
}

// ---------------------------------------------------------------------------
// sys_mprotect — change memory region protection (stub)
// ---------------------------------------------------------------------------

pub unsafe fn sys_mprotect(_addr: u64, _len: usize, _prot: i32) -> i64 {
    // Full permission enforcement requires page table attribute updates.
    // Stubbed to succeed silently for now — sufficient for musl startup.
    0
}

// ---------------------------------------------------------------------------
// sys_access — check file accessibility
// ---------------------------------------------------------------------------

pub unsafe fn sys_access(path_ptr: *const u8, _amode: i32) -> i64 {
    if path_ptr.is_null() {
        return EINVAL;
    }
    let mut buf = [0u8; 512];
    let _len = match copy_user_cstr(path_ptr, &mut buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let path = match core::str::from_utf8(&buf[.._len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    // Check if the file exists (permission bits not yet tracked).
    let exists = crate::fs::vfs_resolve(path, None).is_ok()
        || crate::fs::ramfs_find(path).is_some();
    if exists { 0 } else { ENOENT }
}

// ---------------------------------------------------------------------------
// sys_clock_nanosleep — sleep with a clock-based absolute or relative timeout
// ---------------------------------------------------------------------------

pub unsafe fn sys_clock_nanosleep(
    _clk_id: i32,
    _flags: i32,
    req: *const u64,
    rem: *mut u64,
) -> i64 {
    // Delegate to nanosleep for all clock IDs and modes for now.
    super::time::sys_nanosleep(req, rem)
}

// ---------------------------------------------------------------------------
// sys_clock_getres — return timer resolution
// ---------------------------------------------------------------------------

pub unsafe fn sys_clock_getres(_clk_id: i32, res: *mut u64) -> i64 {
    if res.is_null() {
        return 0; // query-only: permitted
    }
    if !validate_user_pointer(res as u64, 2 * core::mem::size_of::<u64>()) {
        return EFAULT;
    }
    // Report 10ms resolution (one scheduler tick).
    // struct timespec { tv_sec=0, tv_nsec=10_000_000 }
    *res = 0;
    *res.add(1) = 10_000_000;
    0
}

// ---------------------------------------------------------------------------
// sys_waitid — POSIX waitid(2)
// ---------------------------------------------------------------------------

pub unsafe fn sys_waitid(
    which: i32,
    id: i32,
    infop: *mut u8,
    options: i32,
    _rusage: u64,
) -> i64 {
    // Delegate to the existing wait mechanism.
    // which: P_PID=1, P_PGID=2, P_ALL=0
    let pid_arg = match which {
        0 => -1i32,           // P_ALL
        1 => id,              // P_PID
        _ => return EINVAL,
    };
    let mut status: i32 = 0;
    // Pass WNOHANG=0 (blocking), _rusage=0 (not tracked).
    let result = super::process::sys_wait(pid_arg, &mut status as *mut i32, options, 0);
    if result < 0 {
        return result;
    }
    // Fill siginfo_t structure if requested.
    // Layout: si_signo(4), si_errno(4), si_code(4), pad(4), si_pid(8), si_uid(8), si_status(4)
    // We write a minimal version: CLD_EXITED=1, si_pid=reaped_pid, si_status=exit_code.
    if !infop.is_null() && validate_user_pointer(infop as u64, 128) {
        let exit_code = (status >> 8) & 0xFF;
        let out = core::slice::from_raw_parts_mut(infop, 128);
        out.fill(0);
        // si_signo = SIGCHLD (17)
        out[0..4].copy_from_slice(&17i32.to_le_bytes());
        // si_code = CLD_EXITED (1) at offset 8
        out[8..12].copy_from_slice(&1i32.to_le_bytes());
        // si_pid at offset 16
        out[16..24].copy_from_slice(&(result as u64).to_le_bytes());
        // si_status at offset 28
        out[28..32].copy_from_slice(&(exit_code as i32).to_le_bytes());
    }
    let _ = options;
    0
}

// ---------------------------------------------------------------------------
// sys_tkill — send signal to a specific thread
// ---------------------------------------------------------------------------

pub unsafe fn sys_tkill(tid: i32, sig: i32) -> i64 {
    super::signals::sys_kill(tid, sig)
}

// ---------------------------------------------------------------------------
// sys_tgkill — send signal to a thread in a thread group
// ---------------------------------------------------------------------------

pub unsafe fn sys_tgkill(_tgid: i32, tid: i32, sig: i32) -> i64 {
    // For single-threaded processes, tgid check is not enforced.
    super::signals::sys_kill(tid, sig)
}

// ---------------------------------------------------------------------------
// sys_mremap — remap a virtual memory region
//
// Full page-table remap is not yet implemented (requires a VM remap API).
// Returns ENOSYS per POSIX when the feature is absent.
// ---------------------------------------------------------------------------

pub unsafe fn sys_mremap(_old_addr: u64, _old_size: u64, _new_size: u64, _flags: i32, _new_addr: u64) -> i64 {
    ENOSYS
}

// ---------------------------------------------------------------------------
// sys_madvise — advise the kernel about memory usage patterns
//
// Bazzulto uses demand paging without a page cache; all hints are no-ops.
// Returns 0 per POSIX (advisory, not mandatory).
// ---------------------------------------------------------------------------

pub unsafe fn sys_madvise(_addr: u64, _len: usize, _advice: i32) -> i64 { 0 }

// ---------------------------------------------------------------------------
// sys_msync — flush memory-mapped changes to the backing store
//
// Bazzulto does not yet have a dirty-page writeback path for file-backed mmaps.
// Returns 0 (success) — the pages will be consistent on next VFS access anyway.
// ---------------------------------------------------------------------------

pub unsafe fn sys_msync(_addr: u64, _len: usize, _flags: i32) -> i64 { 0 }

// ---------------------------------------------------------------------------
// sys_symlink — create a symbolic link
// ---------------------------------------------------------------------------

pub unsafe fn sys_symlink(target_ptr: *const u8, linkpath_ptr: *const u8) -> i64 {
    if target_ptr.is_null() || linkpath_ptr.is_null() {
        return EINVAL;
    }
    let mut target_buf = [0u8; 512];
    let target_len = match copy_user_cstr(target_ptr, &mut target_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let mut link_buf = [0u8; 512];
    let link_len = match copy_user_cstr(linkpath_ptr, &mut link_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let target = match core::str::from_utf8(&target_buf[..target_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let linkpath = match core::str::from_utf8(&link_buf[..link_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let abs_link = super::vfs::resolve_to_absolute(linkpath);
    let (parent, link_name) = match crate::fs::vfs_resolve_parent(&abs_link) {
        Ok(pair) => pair,
        Err(err) => return err.to_errno(),
    };
    // Allocate a SymlinkInode pointing at `target`.
    // SymlinkInode::new returns Arc<SymlinkInode>; coerce to Arc<dyn Inode> for link_child.
    let symlink_inode = crate::fs::SymlinkInode::new(alloc::string::String::from(target));
    let symlink_dyn: alloc::sync::Arc<dyn crate::fs::inode::Inode> = symlink_inode;
    match parent.link_child(&link_name, symlink_dyn) {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

// ---------------------------------------------------------------------------
// sys_link — create a hard link
//
// Hard links require both inodes to live on the same filesystem and the
// filesystem to track link counts.  FAT32 does not support hard links.
// Returns ENOSYS until the VFS link-count layer is implemented.
// ---------------------------------------------------------------------------

pub unsafe fn sys_link(_oldpath: *const u8, _newpath: *const u8) -> i64 { ENOSYS }

// ---------------------------------------------------------------------------
// sys_readlinkat — read a symbolic link's target relative to a dirfd
// ---------------------------------------------------------------------------

pub unsafe fn sys_readlinkat(
    _dirfd: i32,
    path_ptr: *const u8,
    buf: *mut u8,
    bufsiz: usize,
) -> i64 {
    sys_readlink(path_ptr, buf, bufsiz)
}

// ---------------------------------------------------------------------------
// sys_readlink — read a symbolic link target
// ---------------------------------------------------------------------------

pub unsafe fn sys_readlink(path_ptr: *const u8, buf_ptr: *mut u8, bufsiz: usize) -> i64 {
    if path_ptr.is_null() || buf_ptr.is_null() || bufsiz == 0 {
        return EINVAL;
    }
    if !validate_user_pointer(buf_ptr as u64, bufsiz) {
        return EFAULT;
    }
    let mut path_buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut path_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let path = match core::str::from_utf8(&path_buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let abs_path = super::vfs::resolve_to_absolute(path);
    let inode = match crate::fs::vfs_resolve(&abs_path, None) {
        Ok(i) => i,
        Err(_) => return ENOENT,
    };
    if inode.inode_type() != crate::fs::InodeType::Symlink {
        return EINVAL; // EINVAL: not a symlink (POSIX requires EINVAL here)
    }
    // Read the symlink target by reading the inode's data.
    let mut target_buf = alloc::vec![0u8; bufsiz];
    let read_len = inode.read_at(0, &mut target_buf).unwrap_or(0);
    let copy_len = read_len.min(bufsiz);
    core::ptr::copy_nonoverlapping(target_buf.as_ptr(), buf_ptr, copy_len);
    copy_len as i64
}

// ---------------------------------------------------------------------------
// sys_fchownat / sys_fchmodat — change ownership/mode relative to a dirfd
//
// POSIX permission storage is not yet fully implemented in the VFS layer.
// Returns 0 (success) so tools like tar/cp do not error out on permission calls.
// ---------------------------------------------------------------------------

pub unsafe fn sys_fchownat(_dirfd: i32, _path: *const u8, _uid: u32, _gid: u32, _flags: i32) -> i64 { 0 }
pub unsafe fn sys_fchmodat(_dirfd: i32, _path: *const u8, _mode: u32, _flags: i32) -> i64 { 0 }

// ---------------------------------------------------------------------------
// sys_fchdir — change working directory to a directory fd
// ---------------------------------------------------------------------------

pub unsafe fn sys_fchdir(fd: i32) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    // Extract the inode from the open file descriptor.
    // The cwd_path cannot be reliably reconstructed from an inode alone without
    // a reverse path table, so we set cwd_path to the inode number as a synthetic
    // path.  Tools that need the real path string (e.g. getcwd) will see this
    // value until the next chdir via a string path.
    let inode_opt: Option<(alloc::sync::Arc<dyn crate::fs::Inode>, u64)> =
        crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                None => None,
                Some(process) => {
                    let table = process.file_descriptor_table.lock();
                    match table.get(fd as usize) {
                        Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) => {
                            Some((alloc::sync::Arc::clone(inode), inode.stat().inode_number))
                        }
                        _ => None,
                    }
                }
            }
        });

    let (inode, ino_num) = match inode_opt {
        Some(pair) => pair,
        None => return EBADF,
    };

    if inode.inode_type() != crate::fs::InodeType::Directory {
        return -20; // ENOTDIR
    }

    // Try to find the mount point path for this inode via the VFS mount table.
    // vfs_for_each_mount iterates all mounts; we look for one whose root inode
    // number matches ours to reconstruct a path prefix.
    let mut found_path: Option<alloc::string::String> = None;
    crate::fs::vfs_for_each_mount(|mountpoint, _source, _fstype, root_inode| {
        if found_path.is_none() && root_inode.stat().inode_number == ino_num {
            found_path = Some(alloc::string::String::from(mountpoint));
        }
    });
    let cwd_path = found_path
        .unwrap_or_else(|| alloc::format!("/.inode/{}", ino_num));

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.cwd = Some(inode);
            process.cwd_path = cwd_path;
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_statx — extended file status
//
// Returns a statx struct with the fields that Bazzulto tracks.
// Missing fields (timestamps, block size, dev) are zeroed.
//
// Reference: Linux statx(2), struct statx layout (kernel/include/uapi/linux/stat.h).
// ---------------------------------------------------------------------------

/// Minimum statx mask bits Bazzulto can satisfy.
const STATX_TYPE:  u32 = 0x0001;
const STATX_MODE:  u32 = 0x0002;
const STATX_NLINK: u32 = 0x0004;
const STATX_UID:   u32 = 0x0008;
const STATX_GID:   u32 = 0x0010;
const STATX_SIZE:  u32 = 0x0200;
const STATX_INO:   u32 = 0x0100;

pub unsafe fn sys_statx(
    _dirfd: i32,
    path_ptr: *const u8,
    _flags: i32,
    _mask: u32,
    buf: *mut u8,
) -> i64 {
    // struct statx is 256 bytes on Linux aarch64.
    const STATX_SIZE_BYTES: usize = 256;

    if path_ptr.is_null() || buf.is_null() {
        return EINVAL;
    }
    if !validate_user_pointer(buf as u64, STATX_SIZE_BYTES) {
        return EFAULT;
    }

    let mut path_buf = [0u8; 512];
    let path_len = match copy_user_cstr(path_ptr, &mut path_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let path = match core::str::from_utf8(&path_buf[..path_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };
    let abs_path = super::vfs::resolve_to_absolute(path);
    let inode = match crate::fs::vfs_resolve(&abs_path, None) {
        Ok(i) => i,
        Err(_) => match crate::fs::ramfs_find(&abs_path) {
            Some(data) => {
                // Synthesise a minimal statx for ramfs files.
                let statx_out = core::slice::from_raw_parts_mut(buf, STATX_SIZE_BYTES);
                statx_out.fill(0);
                // stx_mask: SIZE | TYPE | MODE
                let mask = STATX_SIZE | STATX_TYPE | STATX_MODE;
                statx_out[0..4].copy_from_slice(&mask.to_le_bytes());
                // stx_size at offset 40
                let size = data.len() as u64;
                statx_out[40..48].copy_from_slice(&size.to_le_bytes());
                // stx_mode at offset 28 (regular file, 0644)
                let mode: u16 = 0o100644;
                statx_out[28..30].copy_from_slice(&mode.to_le_bytes());
                return 0;
            }
            None => return ENOENT,
        },
    };

    let stat = inode.stat();
    let statx_out = core::slice::from_raw_parts_mut(buf, STATX_SIZE_BYTES);
    statx_out.fill(0);

    // stx_mask: everything we can fill
    let mask = STATX_TYPE | STATX_MODE | STATX_NLINK | STATX_UID | STATX_GID | STATX_SIZE | STATX_INO;
    statx_out[0..4].copy_from_slice(&mask.to_le_bytes());

    // stx_nlink at offset 16
    statx_out[16..20].copy_from_slice(&(stat.nlinks as u32).to_le_bytes());

    // stx_uid, stx_gid at offsets 20, 24
    // (all files owned by uid=0 until per-inode ownership is tracked)
    statx_out[20..24].copy_from_slice(&0u32.to_le_bytes()); // stx_uid
    statx_out[24..28].copy_from_slice(&0u32.to_le_bytes()); // stx_gid

    // stx_mode at offset 28 (includes file type bits from InodeStat::mode)
    let mode = stat.mode as u16;
    statx_out[28..30].copy_from_slice(&mode.to_le_bytes());

    // stx_ino at offset 32
    statx_out[32..40].copy_from_slice(&stat.inode_number.to_le_bytes());

    // stx_size at offset 40
    statx_out[40..48].copy_from_slice(&stat.size.to_le_bytes());

    0
}

// ---------------------------------------------------------------------------
// sys_readv — scatter-gather read
//
// struct iovec { void *iov_base; size_t iov_len; }  — two u64 fields on AArch64.
// ---------------------------------------------------------------------------

pub unsafe fn sys_readv(fd: i32, iov_ptr: u64, iovcnt: i32) -> i64 {
    if fd < 0 || iovcnt < 0 || iovcnt > 1024 {
        return EINVAL;
    }
    if iov_ptr == 0 {
        return EINVAL;
    }
    let iov_count = iovcnt as usize;
    // Each iovec is two u64 words = 16 bytes.
    if !validate_user_pointer(iov_ptr, iov_count * 16) {
        return EFAULT;
    }
    let mut total_read: i64 = 0;
    for i in 0..iov_count {
        let entry_ptr = (iov_ptr + (i * 16) as u64) as *const u64;
        let base = core::ptr::read(entry_ptr);
        let len  = core::ptr::read(entry_ptr.add(1)) as usize;
        if len == 0 {
            continue;
        }
        if !validate_user_pointer(base, len) {
            return EFAULT;
        }
        let result = super::io::sys_read(fd, base as *mut u8, len);
        if result < 0 {
            if total_read == 0 { return result; }
            break; // partial read — return bytes read so far
        }
        total_read += result;
        if (result as usize) < len {
            break; // short read — stop (EOF or pipe empty)
        }
    }
    total_read
}

// ---------------------------------------------------------------------------
// sys_writev — scatter-gather write
// ---------------------------------------------------------------------------

pub unsafe fn sys_writev(fd: i32, iov_ptr: u64, iovcnt: i32) -> i64 {
    if fd < 0 || iovcnt < 0 || iovcnt > 1024 {
        return EINVAL;
    }
    if iov_ptr == 0 {
        return EINVAL;
    }
    let iov_count = iovcnt as usize;
    if !validate_user_pointer(iov_ptr, iov_count * 16) {
        return EFAULT;
    }
    let mut total_written: i64 = 0;
    for i in 0..iov_count {
        let entry_ptr = (iov_ptr + (i * 16) as u64) as *const u64;
        let base = core::ptr::read(entry_ptr);
        let len  = core::ptr::read(entry_ptr.add(1)) as usize;
        if len == 0 {
            continue;
        }
        if !validate_user_pointer(base, len) {
            return EFAULT;
        }
        let result = super::io::sys_write(fd, base as *const u8, len);
        if result < 0 {
            if total_written == 0 { return result; }
            break;
        }
        total_written += result;
    }
    total_written
}

// ---------------------------------------------------------------------------
// sys_pread64 — read from fd at a given offset without moving the file position
// ---------------------------------------------------------------------------

pub unsafe fn sys_pread64(fd: i32, buf_ptr: *mut u8, count: usize, offset: u64) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(buf_ptr as u64, count) {
        return EFAULT;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EBADF,
    };
    let guard = fd_table_arc.lock();
    match guard.get(fd as usize) {
        Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) => {
            let dest = core::slice::from_raw_parts_mut(buf_ptr, count);
            match inode.read_at(offset, dest) {
                Ok(n) => n as i64,
                Err(_) => EINVAL,
            }
        }
        Some(crate::fs::vfs::FileDescriptor::RamFsFile { data, .. }) => {
            let start = offset as usize;
            if start >= data.len() {
                return 0;
            }
            let available = &data[start..];
            let copy_len = available.len().min(count);
            let dest = core::slice::from_raw_parts_mut(buf_ptr, copy_len);
            dest.copy_from_slice(&available[..copy_len]);
            copy_len as i64
        }
        Some(_) => ESPIPE, // pipes, ttys: POSIX says ESPIPE for pread on non-seekable
        None => EBADF,
    }
}

// ---------------------------------------------------------------------------
// sys_pwrite64 — write to fd at a given offset without moving the file position
// ---------------------------------------------------------------------------

pub unsafe fn sys_pwrite64(fd: i32, buf_ptr: *const u8, count: usize, offset: u64) -> i64 {
    if fd < 0 {
        return EBADF;
    }
    if !validate_user_pointer(buf_ptr as u64, count) {
        return EFAULT;
    }
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return EBADF,
    };
    let guard = fd_table_arc.lock();
    match guard.get(fd as usize) {
        Some(crate::fs::vfs::FileDescriptor::InoFile { inode, .. }) => {
            let src = core::slice::from_raw_parts(buf_ptr, count);
            match inode.write_at(offset, src) {
                Ok(n) => n as i64,
                Err(_) => EINVAL,
            }
        }
        Some(_) => ESPIPE,
        None => EBADF,
    }
}

// ---------------------------------------------------------------------------
// sys_renameat — rename relative to directory fds
//
// AT_FDCWD: delegates to existing sys_rename.
// Other dirfds: not yet supported (no path reconstruction from open dirfd).
// ---------------------------------------------------------------------------

pub unsafe fn sys_renameat(
    olddirfd: i32,
    oldpath_ptr: *const u8,
    newdirfd: i32,
    newpath_ptr: *const u8,
) -> i64 {
    // Both must be AT_FDCWD or absolute paths for now.
    if (olddirfd != AT_FDCWD && olddirfd >= 0) || (newdirfd != AT_FDCWD && newdirfd >= 0) {
        return EBADF;
    }
    if oldpath_ptr.is_null() || newpath_ptr.is_null() {
        return EINVAL;
    }
    let mut old_buf = [0u8; 512];
    let old_len = match copy_user_cstr(oldpath_ptr, &mut old_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    let mut new_buf = [0u8; 512];
    let new_len = match copy_user_cstr(newpath_ptr, &mut new_buf) {
        Some(l) => l,
        None => return EFAULT,
    };
    super::vfs::sys_rename(old_buf.as_ptr(), new_buf.as_ptr())
}

// ---------------------------------------------------------------------------
// sys_times — return process and child times
//
// struct tms { clock_t tms_utime; tms_stime; tms_cutime; tms_cstime; }
// Each field is a u64 tick count.  Child cumulative times are 0 (not tracked).
//
// Reference: POSIX.1-2017 times(2).
// ---------------------------------------------------------------------------

pub unsafe fn sys_times(buf_ptr: *mut u64) -> i64 {
    if buf_ptr.is_null() || !validate_user_pointer(buf_ptr as u64, 4 * core::mem::size_of::<u64>()) {
        return EFAULT;
    }
    let (user_ticks, sys_ticks) = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| (p.user_ticks, p.sys_time_ticks))
            .unwrap_or((0, 0))
    });
    *buf_ptr            = user_ticks; // tms_utime
    *buf_ptr.add(1)     = sys_ticks;  // tms_stime
    *buf_ptr.add(2)     = 0;          // tms_cutime (children not tracked)
    *buf_ptr.add(3)     = 0;          // tms_cstime
    // Return value: elapsed real time in ticks since an arbitrary epoch.
    crate::platform::qemu_virt::timer::current_tick() as i64
}

// ---------------------------------------------------------------------------
// sys_getgroups — return supplementary group IDs
//
// Bazzulto uses a single-GID model.  Returns 0 groups (all processes have
// egid as their sole group).  Per POSIX: if size == 0, return number of groups.
// ---------------------------------------------------------------------------

pub unsafe fn sys_getgroups(size: i32, _list: *mut u32) -> i64 {
    // We have 0 supplementary groups beyond the primary gid.
    let _ = size;
    0
}

// ---------------------------------------------------------------------------
// sys_getpgid — return process group ID of a process
// ---------------------------------------------------------------------------

pub unsafe fn sys_getpgid_syscall(pid: i32) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        let target_pid = if pid == 0 {
            scheduler.current_pid()
        } else {
            crate::process::Pid::new(pid as u16, 1)
        };
        scheduler.process(target_pid)
            .map(|p| p.pgid as i64)
            .unwrap_or(ESRCH)
    })
}

// ---------------------------------------------------------------------------
// sys_clock_settime — set a clock
//
// Only root can set the realtime clock.  Bazzulto does not support setting
// the hardware RTC from userspace yet.  Returns EPERM for all callers.
// ---------------------------------------------------------------------------

pub unsafe fn sys_clock_settime(_clk_id: i32, _tp: *const u64) -> i64 { EPERM }

// ---------------------------------------------------------------------------
// POSIX interval timers (timer_create / timer_settime / timer_gettime / timer_delete)
//
// A full per-process timer queue requires a heap and signal routing infrastructure
// that does not yet exist.  Returns ENOSYS until implemented.
// ---------------------------------------------------------------------------

pub unsafe fn sys_timer_create(_clk: i32, _evp: u64, _timerid: *mut i32) -> i64 { ENOSYS }
pub unsafe fn sys_timer_settime(_id: i32, _flags: i32, _new: u64, _old: u64) -> i64 { ENOSYS }
pub unsafe fn sys_timer_gettime(_id: i32, _cur: u64) -> i64 { ENOSYS }
pub unsafe fn sys_timer_delete(_id: i32) -> i64 { ENOSYS }

// ---------------------------------------------------------------------------
// sys_setitimer / sys_getitimer — BSD interval timers
//
// ITIMER_REAL (0): mapped to the existing alarm_deadline_tick mechanism.
//   Delivers SIGALRM on expiry.  Interval (it_interval) is not yet supported
//   (the timer does not re-arm automatically).
// ITIMER_VIRTUAL (1), ITIMER_PROF (2): require per-tick user-time tracking;
//   not implemented yet.
//
// struct itimerval { timeval it_interval; timeval it_value; }
// struct timeval   { i64 tv_sec; i64 tv_usec; }
// Each itimerval = 32 bytes (4 × i64).
//
// Reference: POSIX.1-2017 setitimer(2).
// ---------------------------------------------------------------------------

const ITIMER_REAL: i32 = 0;

pub unsafe fn sys_setitimer(which: i32, new_ptr: u64, old_ptr: u64) -> i64 {
    if which != ITIMER_REAL {
        return ENOSYS;
    }
    const TICKS_PER_SECOND: u64 =
        1_000 / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
    let now_tick = crate::platform::qemu_virt::timer::current_tick();

    // Read the new itimerval from userspace (it_interval then it_value, 32 bytes).
    if new_ptr != 0 && !validate_user_pointer(new_ptr, 32) {
        return EFAULT;
    }
    let (new_sec, new_usec) = if new_ptr != 0 {
        let sec  = core::ptr::read((new_ptr + 16) as *const i64); // it_value.tv_sec
        let usec = core::ptr::read((new_ptr + 24) as *const i64); // it_value.tv_usec
        (sec.max(0) as u64, usec.max(0) as u64)
    } else {
        (0, 0)
    };

    crate::scheduler::with_scheduler(|scheduler| {
        let process = match scheduler.current_process_mut() {
            Some(p) => p,
            None => return ESRCH,
        };

        // Write old itimerval if requested.
        if old_ptr != 0 && validate_user_pointer(old_ptr, 32) {
            let remaining_ticks = if process.alarm_deadline_tick > now_tick {
                process.alarm_deadline_tick - now_tick
            } else {
                0
            };
            let rem_ms = remaining_ticks * crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
            let rem_sec  = (rem_ms / 1_000) as i64;
            let rem_usec = ((rem_ms % 1_000) * 1_000) as i64;
            // it_interval = {0, 0} (no auto-repeat yet)
            core::ptr::write((old_ptr +  0) as *mut i64, 0);
            core::ptr::write((old_ptr +  8) as *mut i64, 0);
            // it_value = remaining
            core::ptr::write((old_ptr + 16) as *mut i64, rem_sec);
            core::ptr::write((old_ptr + 24) as *mut i64, rem_usec);
        }

        // Set new deadline.
        if new_sec == 0 && new_usec == 0 {
            process.alarm_deadline_tick = 0; // cancel
        } else {
            let total_ms = new_sec.saturating_mul(1_000)
                .saturating_add(new_usec / 1_000);
            let ticks = (total_ms + crate::platform::qemu_virt::timer::TICK_INTERVAL_MS - 1)
                / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
            process.alarm_deadline_tick = now_tick + ticks;
        }
        0i64
    })
}

pub unsafe fn sys_getitimer(which: i32, cur_ptr: u64) -> i64 {
    if which != ITIMER_REAL {
        return ENOSYS;
    }
    if cur_ptr == 0 || !validate_user_pointer(cur_ptr, 32) {
        return EFAULT;
    }
    let now_tick = crate::platform::qemu_virt::timer::current_tick();
    let remaining_ticks = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| {
                if p.alarm_deadline_tick > now_tick {
                    p.alarm_deadline_tick - now_tick
                } else {
                    0
                }
            })
            .unwrap_or(0)
    });
    let rem_ms   = remaining_ticks * crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
    let rem_sec  = (rem_ms / 1_000) as i64;
    let rem_usec = ((rem_ms % 1_000) * 1_000) as i64;
    // it_interval = {0, 0}
    core::ptr::write((cur_ptr +  0) as *mut i64, 0);
    core::ptr::write((cur_ptr +  8) as *mut i64, 0);
    // it_value
    core::ptr::write((cur_ptr + 16) as *mut i64, rem_sec);
    core::ptr::write((cur_ptr + 24) as *mut i64, rem_usec);
    0
}
