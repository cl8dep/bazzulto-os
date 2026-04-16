// process.rs — Process lifecycle syscall implementations.
//
// Syscalls: exit, fork, exec, getpid, getppid, spawn, wait, clone, gettid

use super::*;

// ---------------------------------------------------------------------------
// sys_exit
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_exit(exit_code: i32) -> i64 {
    let pid = crate::scheduler::with_scheduler(|s| s.current_pid());
    crate::drivers::uart::puts("[exit] pid=");
    crate::drivers::uart::put_hex(pid.index as u64);
    crate::drivers::uart::puts(" code=");
    crate::drivers::uart::put_hex(exit_code as u64);
    crate::drivers::uart::puts("\r\n");
    // Grab the FD table Arc before entering the scheduler critical section.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    // Close all descriptors so pipe write-ends are released promptly.
    // We always close: this is the only place fds are released on exit.
    // The Arc count is at least 2 here (one in the process struct + the
    // clone above), so the old `== 1` guard was always false — a bug that
    // caused pipes to never receive EOF.
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        guard.close_all();
    }
    crate::scheduler::with_scheduler::<_, ()>(|scheduler| {
        scheduler.exit(exit_code);
    });
    // Never reached.
    #[allow(unreachable_code)]
    0
}

// ---------------------------------------------------------------------------
// sys_spawn
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_spawn(name_ptr: *const u8, capability_mask: u64) -> i64 {
    let mut name_buf = [0u8; 512];
    let name_len = match copy_user_cstr(name_ptr, &mut name_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let name = match core::str::from_utf8(&name_buf[..name_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Try VFS first (FAT32 disk), fall back to legacy ramfs for built-ins.
    // This mirrors the pattern used by sys_exec.
    // Also collect cwd_path so the child inherits the working directory.
    let (cwd, parent_cwd_path) = crate::scheduler::with_scheduler(|scheduler| {
        let cwd      = scheduler.current_process().and_then(|p| p.cwd.clone());
        let cwd_path = scheduler.current_process()
            .map(|p| p.cwd_path.clone())
            .unwrap_or_else(|| alloc::string::String::from("/"));
        (cwd, cwd_path)
    });
    let vfs_inode = crate::fs::vfs_resolve(name, cwd.as_ref()).ok();

    // DAC execute permission check (POSIX.1-2017 exec(2)).
    if let Some(ref inode) = vfs_inode {
        let denied = crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => crate::fs::vfs_check_access(
                    &inode.stat(), p.euid, p.egid,
                    &p.supplemental_groups, p.ngroups,
                    crate::fs::ACCESS_EXECUTE,
                ).is_err(),
                None => false, // kernel boot context — no check needed
            }
        });
        if denied { return EACCES; }
    }

    let owned_elf: alloc::vec::Vec<u8>;
    let elf_data: &[u8] = if let Some(ref inode) = vfs_inode {
        let size = inode.stat().size as usize;
        owned_elf = {
            let mut buf = alloc::vec![0u8; size];
            let _ = inode.read_at(0, &mut buf);
            buf
        };
        &owned_elf
    } else if let Some(data) = crate::fs::ramfs_find(name) {
        data
    } else {
        return ENOENT;
    };

    // Collect the parent's environ so the child inherits it on the initial stack.
    // We build a Vec<Vec<u8>> of "KEY=VALUE" byte strings first, then build
    // &[&[u8]] slices for load_elf.
    let parent_environ: alloc::vec::Vec<alloc::vec::Vec<u8>> =
        crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => p.environ.iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect(),
                None => alloc::vec::Vec::new(),
            }
        });
    let envp_slices: alloc::vec::Vec<&[u8]> =
        parent_environ.iter().map(|v| v.as_slice()).collect();

    // Load the ELF into a new address space.
    let loaded = crate::memory::with_physical_allocator(|phys| {
        crate::loader::load_elf(elf_data, phys, &[], &envp_slices)
    });

    let loaded = match loaded {
        Ok(image) => image,
        Err(_) => return ENOMEM,
    };
    // Box the page table outside of with_physical_allocator to avoid
    // re-entrant access to the physical allocator via GlobalAlloc.
    let loaded_page_table = alloc::boxed::Box::new(loaded.page_table);

    // Create the new process.
    let child_pid = crate::scheduler::with_scheduler(|scheduler| {
        let parent_pid = scheduler.current_pid();

        // Capability check: if the caller is requesting capabilities, it must
        // hold CAP_SETCAP.  It may only grant capabilities it already holds.
        if capability_mask != 0 {
            let parent_caps = scheduler.current_process()
                .map(|p| p.capabilities)
                .unwrap_or(0);

            if parent_caps & crate::process::CAP_SETCAP == 0 {
                return None; // caller lacks CAP_SETCAP → EPERM
            }
            // Cannot grant capabilities not held by the parent.
            if capability_mask & !parent_caps != 0 {
                return None; // EPERM
            }
        }

        // Clone the parent's current fd table so the child inherits open fds
        // (including any pipe ends that bzinit redirected via dup2 before spawn).
        let (parent_fd_table_clone, parent_identity) = match scheduler.current_process() {
            Some(p) => {
                let fd_clone = {
                    let guard = p.file_descriptor_table.lock();
                    guard.clone_for_fork()
                };
                let identity = (p.uid, p.gid, p.euid, p.egid, p.suid, p.sgid,
                                p.supplemental_groups, p.ngroups);
                (Some(fd_clone), Some(identity))
            }
            None => (None, None),
        };

        let child_pid = match scheduler.create_process(Some(parent_pid)) {
            Some(pid) => pid,
            None => return None,
        };

        let child = match scheduler.process_mut(child_pid) {
            Some(process) => process,
            None => return None,
        };

        // Install the new page table.
        child.page_table = Some(loaded_page_table);
        // Register the demand-paged stack region.
        let stack_region = crate::process::MmapRegion {
            base:   loaded.stack_demand_base,
            length: loaded.stack_demand_top - loaded.stack_demand_base,
            demand: true,
            shared: false,
            backing: crate::process::MmapBacking::Anonymous,
        };
        child.mmap_regions.insert(loaded.stack_demand_base, stack_region);
        // Grant requested capabilities (already validated above).
        child.capabilities = capability_mask;
        // Inherit the parent's environment.
        child.environ = parent_environ.iter()
            .filter_map(|v| core::str::from_utf8(v).ok().map(|s| alloc::string::String::from(s)))
            .collect();

        // Inherit the parent's working directory.
        child.cwd      = cwd.clone();
        child.cwd_path = parent_cwd_path.clone();

        // POSIX identity: child inherits parent's UID/GID fields.
        if let Some((uid, gid, euid, egid, suid, sgid, groups, ngroups)) = parent_identity {
            child.uid = uid;
            child.gid = gid;
            child.euid = euid;
            child.egid = egid;
            child.suid = suid;
            child.sgid = sgid;
            child.supplemental_groups = groups;
            child.ngroups = ngroups;
        }

        // Replace the fresh fd table with the parent's clone (inheriting pipes).
        if let Some(fd_clone) = parent_fd_table_clone {
            *child.file_descriptor_table.lock() = fd_clone;
        }

        // Build the initial ExceptionFrame on the child's kernel stack.
        let frame_ptr = (child.kernel_stack.top as usize
            - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
        core::ptr::write_bytes(frame_ptr as *mut u8, 0, core::mem::size_of::<ExceptionFrame>());
        // ELR_EL1 = entry point, SPSR_EL1 = EL0 (0x0), SP_EL0 = user stack.
        (*frame_ptr).elr = loaded.entry_point;
        (*frame_ptr).spsr = 0; // EL0 state
        (*frame_ptr).sp = loaded.initial_stack_pointer;
        crate::uart::puts("[spawn] sp=");
        crate::uart::put_hex(loaded.initial_stack_pointer);
        crate::uart::puts(" demand=[");
        crate::uart::put_hex(loaded.stack_demand_base);
        crate::uart::puts(", ");
        crate::uart::put_hex(loaded.stack_demand_top);
        crate::uart::puts(")\r\n");
        // AArch64 SYSV ABI: x0 = argc, x1 = argv[] VA, x2 = envp[] VA.
        (*frame_ptr).x[0] = loaded.argc as u64;
        (*frame_ptr).x[1] = loaded.argv_va;
        (*frame_ptr).x[2] = loaded.envp_va;

        child.cpu_context.stack_pointer = frame_ptr as u64;
        child.cpu_context.link_register = crate::process::process_entry_trampoline_el0 as *const () as u64;

        // Mark the new process as foreground.
        child.is_foreground = true;

        scheduler.make_ready(child_pid);
        Some(child_pid)
    });

    match child_pid {
        Some(pid) => pid.index as i64,
        None => ENOMEM,
    }
}

// ---------------------------------------------------------------------------
// sys_wait — wait for a child process to exit
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_wait(pid_arg: i32, status_ptr: *mut i32, options: i32, _rusage: u64) -> i64 {
    // WNOHANG: do not block if no child has changed state yet.
    // Linux value: 1.  Reference: POSIX.1-2017 waitpid(2) WNOHANG.
    const WNOHANG: i32 = 1;

    let for_pid = if pid_arg < 0 {
        None // wait for any child
    } else {
        Some(crate::process::Pid::new(pid_arg as u16, 1))
    };

    // POSIX.1-2017 wait(2): if the calling process has no existing unwaited-for
    // child processes, return ECHILD immediately rather than blocking forever.
    let has_any_children = crate::scheduler::with_scheduler(|scheduler| {
        let current_pid = scheduler.current_pid();
        scheduler.has_children(current_pid)
    });
    if !has_any_children {
        return ECHILD;
    }

    loop {
        let result = crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            scheduler.reap(current_pid, for_pid)
        });

        if let Some((reaped_pid, exit_code)) = result {
            if !status_ptr.is_null() && (status_ptr as u64) < crate::process::USER_ADDR_LIMIT {
                // POSIX.1-2017 waitpid(2): encode exit status so that the
                // standard WIFEXITED(status) and WEXITSTATUS(status) macros
                // work correctly.  Normal termination encodes exit_code in
                // bits [15:8] with bits [7:0] == 0.
                // Reference: POSIX.1-2017 §2.13, sys/wait.h macros.
                *status_ptr = (exit_code & 0xFF) << 8;
            }
            return reaped_pid.index as i64;
        }

        // WNOHANG: return 0 immediately if no zombie child is ready.
        // Reference: POSIX.1-2017 waitpid(2) — WNOHANG causes return of 0
        // when no child has changed state yet.
        if options & WNOHANG != 0 {
            return 0;
        }

        // No zombie child yet — block until one exits.
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process_mut() {
                process.state = crate::process::ProcessState::Waiting { for_pid };
            }
            scheduler.schedule_no_requeue();
        });
    }
}

// ---------------------------------------------------------------------------
// sys_pipe — create a pipe
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_fork
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_fork(frame: *mut ExceptionFrame) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.fork(frame) {
            Ok(child_pid) => child_pid.index as i64,
            // POSIX.1-2017 fork(2): EAGAIN if process/resource limit reached,
            // ENOMEM if insufficient memory.  InternalError is an OOM-class
            // failure, not a "process not found" situation, so ESRCH is wrong.
            Err(crate::scheduler::ForkError::OutOfPids) => EAGAIN,
            Err(crate::scheduler::ForkError::OutOfMemory) => ENOMEM,
            Err(crate::scheduler::ForkError::InternalError) => ENOMEM,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_exec — replace current process image with a VFS or ramfs binary
// ---------------------------------------------------------------------------

/// Parse a null-separated flat byte buffer into a fixed-size slice array.
///
/// Format: `"entry0\0entry1\0entry2\0"` — each entry ends at a NUL byte.
/// Entries that are not valid UTF-8 are silently skipped.
/// At most `N` entries are stored; excess entries are silently dropped.

// ---------------------------------------------------------------------------
// parse_flat_strings helper
// ---------------------------------------------------------------------------

unsafe fn parse_flat_strings<'a, const N: usize>(
    ptr: *const u8,
    length: usize,
    out: &mut [&'a [u8]; N],
) -> usize {
    let mut count = 0usize;
    if ptr.is_null() || length == 0 || (ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return 0;
    }
    let flat = core::slice::from_raw_parts(ptr, length);
    let mut start = 0usize;
    for (index, &byte) in flat.iter().enumerate() {
        if byte == 0 {
            if index > start && count < N {
                let slice = &flat[start..index];
                if core::str::from_utf8(slice).is_ok() {
                    out[count] = slice;
                    count += 1;
                }
            }
            start = index + 1;
        }
    }
    count
}


// ---------------------------------------------------------------------------
// read_user_string_array — read a NULL-terminated argv/envp pointer array
// ---------------------------------------------------------------------------

/// Read a NULL-terminated array of NUL-terminated C string pointers from user space.
///
/// `array_ptr` is the VA of the first element (a `*const *const u8` in C terms).
/// Each element is a 64-bit pointer to a NUL-terminated string.
/// The array is terminated by a NULL (0) pointer.
/// At most `MAX` entries are read; excess are silently dropped.
///
/// Returns the count of valid entries written.  The caller builds slices from
/// `string_bufs` after this function returns (two-pass design to satisfy the
/// borrow checker: write buffers first, then borrow them immutably).
///
/// # Safety
/// All pointers are validated against USER_ADDR_LIMIT before access.
unsafe fn read_user_string_array_into<const MAX: usize>(
    array_ptr: u64,
    string_bufs: &mut [[u8; 256]; MAX],
    string_lens: &mut [usize; MAX],
) -> usize {
    if array_ptr == 0 || array_ptr >= crate::process::USER_ADDR_LIMIT {
        return 0;
    }
    let mut count = 0usize;
    let mut index = 0usize;
    while index < MAX {
        let ptr_addr = array_ptr + (index as u64).wrapping_mul(8);
        if ptr_addr + 8 > crate::process::USER_ADDR_LIMIT {
            break;
        }
        let str_ptr = core::ptr::read_volatile(ptr_addr as *const u64);
        if str_ptr == 0 {
            // NULL terminator — end of array.
            break;
        }
        if str_ptr >= crate::process::USER_ADDR_LIMIT {
            // Skip invalid (out-of-range) pointer; do not advance count.
            index += 1;
            continue;
        }
        // Write all bytes before any immutable borrows of the buffer.
        let mut len = 0usize;
        while len < 255 {
            let byte = core::ptr::read_volatile((str_ptr + len as u64) as *const u8);
            string_bufs[count][len] = byte;
            if byte == 0 {
                break;
            }
            len += 1;
        }
        string_bufs[count][len] = 0; // ensure NUL termination
        string_lens[count] = len;
        count += 1;
        index += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// sys_exec
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_exec(
    frame: *mut ExceptionFrame,
    name_ptr: *const u8,
    argv_ptr: u64,
    envp_ptr: u64,
) -> i64 {
    let mut name_buf = [0u8; 512];
    let name_len = match copy_user_cstr(name_ptr, &mut name_buf) {
        Some(l) => l,
        None => return EINVAL,
    };
    let name = match core::str::from_utf8(&name_buf[..name_len]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Resolve via VFS first; fall back to legacy ramfs.
    let vfs_inode = crate::fs::vfs_resolve(name, None).ok();
    let ramfs_data = if vfs_inode.is_none() { crate::fs::ramfs_find(name) } else { None };

    // INODE_KERNEL_EXEC_ONLY check: reject userspace exec of kernel-only binaries.
    if crate::fs::vfs_is_kernel_exec_only(name) {
        return EPERM;
    }

    // DAC execute permission check (POSIX.1-2017 exec(2)).
    // The caller must have execute permission on the binary.
    if let Some(ref inode) = vfs_inode {
        let denied = crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.current_process() {
                Some(p) => crate::fs::vfs_check_access(
                    &inode.stat(), p.euid, p.egid,
                    &p.supplemental_groups, p.ngroups,
                    crate::fs::ACCESS_EXECUTE,
                ).is_err(),
                None => false,
            }
        });
        if denied { return EACCES; }
    }

    // We need owned data for the VFS path because the inode may be backed by a
    // Vec<u8> whose lifetime is tied to the inode Arc — we copy it to avoid
    // borrowing through the scheduler lock.
    let owned_elf: alloc::vec::Vec<u8>;
    let elf_data: &[u8] = if let Some(ref inode) = vfs_inode {
        let size = inode.stat().size as usize;
        owned_elf = {
            let mut buf = alloc::vec![0u8; size];
            let _ = inode.read_at(0, &mut buf);
            buf
        };
        &owned_elf
    } else if let Some(data) = ramfs_data {
        data
    } else {
        return ENOENT;
    };

    // Binary Permission Model — tier dispatch at exec time.
    //
    // Tier 1: system binary → full trust (wildcard permissions).
    // Tier 4: no .bazzulto_permissions section → inherit from parent + warn.
    // Tier 2/3: section present → permissiond handles it (post-v1.0); we do
    //           not touch the sets and leave them as-is.
    //
    // Reference: docs/features/Binary Permission Model.md §Tier Dispatch.
    let exec_permission_tier_result = {
        let has_perm_section = crate::permission::elf_has_bazzulto_permissions_section(elf_data);
        let (parent_perms, parent_actions) = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| (p.granted_permissions.clone(), p.granted_actions.clone()))
                .unwrap_or_default()
        });
        crate::permission::resolve_exec_permissions(
            name,
            has_perm_section,
            &parent_perms,
            &parent_actions,
            name,
        )
    };

    // Parse argv: NULL-terminated array of NUL-terminated C string pointers.
    // Two-pass: first write all strings into fixed buffers (read_user_string_array_into),
    // then build immutable slices.  Cap at 64 entries to bound kernel stack usage.
    let mut argv_bufs = [[0u8; 256]; 64];
    let mut argv_lens = [0usize; 64];
    let argv_count = read_user_string_array_into::<64>(argv_ptr, &mut argv_bufs, &mut argv_lens);
    let argv: alloc::vec::Vec<&[u8]> = (0..argv_count)
        .map(|i| &argv_bufs[i][..argv_lens[i]])
        .collect();

    // Parse envp: NULL-terminated array of NUL-terminated C string pointers.
    // Cap at 128 entries.
    let mut envp_bufs = [[0u8; 256]; 128];
    let mut envp_lens = [0usize; 128];
    let envp_count = read_user_string_array_into::<128>(envp_ptr, &mut envp_bufs, &mut envp_lens);
    let envp: alloc::vec::Vec<&[u8]> = (0..envp_count)
        .map(|i| &envp_bufs[i][..envp_lens[i]])
        .collect();

    let loaded = crate::memory::with_physical_allocator(|phys| {
        crate::loader::load_elf(elf_data, phys, &argv, &envp)
    });

    let loaded = match loaded {
        Ok(image) => image,
        Err(crate::loader::LoaderError::NotAnElf)
        | Err(crate::loader::LoaderError::UnsupportedFormat)
        | Err(crate::loader::LoaderError::NotExecutable)
        | Err(crate::loader::LoaderError::UnalignedSegment)
        | Err(crate::loader::LoaderError::Truncated) => {
            // POSIX.1-2017 exec(2) §ERRORS: ENOEXEC if the file has the
            // appropriate access permission but an unrecognised format.
            return ENOEXEC;
        }
        Err(_) => return ENOMEM,
    };
    // Box the page table outside with_physical_allocator to avoid re-entrant
    // access to the physical allocator via GlobalAlloc.
    let loaded_page_table = alloc::boxed::Box::new(loaded.page_table);

    // Close cloexec FDs before replacing the address space.
    // Do this outside the scheduler lock to avoid holding two locks simultaneously.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        // Iterate all 16 words of the cloexec bitmask and close every marked fd.
        for word_index in 0..16usize {
            let mut word = guard.cloexec_mask[word_index];
            while word != 0 {
                let bit_pos = word.trailing_zeros() as usize;
                let fd_number = word_index * 64 + bit_pos;
                guard.close(fd_number);
                word &= word - 1;
            }
        }
        guard.cloexec_mask = [0u64; 16];
        guard.nonblock_mask = [0u64; 16];
    }

    // Setuid/setgid on exec: check inode mode bits and adjust identity.
    //
    // POSIX.1-2017 exec(2): if the new program file has the S_ISUID bit set,
    // the effective UID of the process is set to the owner of the file.
    // Similarly for S_ISGID.  The saved UID/GID are set to the new effective
    // values.  If neither bit is set, suid/sgid are set to euid/egid (clearing
    // any previous saved set-id state).
    let setuid_identity: Option<(u32, u32, u32, u32)> = vfs_inode.as_ref().map(|inode| {
        let stat = inode.stat();
        let mode = stat.mode;
        let s_isuid = mode & 0o4000 != 0;
        let s_isgid = mode & 0o2000 != 0;
        let new_euid = if s_isuid { stat.uid } else { u32::MAX }; // MAX = no change
        let new_egid = if s_isgid { stat.gid } else { u32::MAX };
        let new_suid = if s_isuid { stat.uid } else { u32::MAX }; // will be set to euid
        let new_sgid = if s_isgid { stat.gid } else { u32::MAX };
        (new_euid, new_egid, new_suid, new_sgid)
    });

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {

            // Apply setuid/setgid identity changes.
            if let Some((new_euid, new_egid, new_suid, new_sgid)) = setuid_identity {
                if new_euid != u32::MAX {
                    process.euid = new_euid;
                    process.suid = new_euid;
                } else {
                    // No setuid: saved UID becomes current effective UID.
                    process.suid = process.euid;
                }
                if new_egid != u32::MAX {
                    process.egid = new_egid;
                    process.sgid = new_egid;
                } else {
                    process.sgid = process.egid;
                }
            }

            // Replace address space.
            process.page_table = Some(loaded_page_table);
            process.mmap_next_va = crate::process::MMAP_USER_BASE;
            process.mmap_regions.clear();
            // Register the demand-paged stack region for the new image.
            let stack_region = crate::process::MmapRegion {
                base:   loaded.stack_demand_base,
                length: loaded.stack_demand_top - loaded.stack_demand_base,
                demand: true,
                shared: false,
                backing: crate::process::MmapBacking::Anonymous,
            };
            process.mmap_regions.insert(loaded.stack_demand_base, stack_region);

            // Apply Binary Permission Model tier result.
            // Tier 1/4: replace both sets.
            // Tier 2/3 (None): leave sets untouched — permissiond will set them.
            if let Some((new_perms, new_actions)) = exec_permission_tier_result.clone() {
                process.granted_permissions = new_perms;
                process.granted_actions = new_actions;
            }

            // Patch the exception frame in-place to redirect to the new entry.
            (*frame).elr = loaded.entry_point;
            (*frame).spsr = 0; // EL0 state
            (*frame).sp = loaded.initial_stack_pointer;
            crate::uart::puts("[exec] sp=");
            crate::uart::put_hex(loaded.initial_stack_pointer);
            crate::uart::puts(" demand=[");
            crate::uart::put_hex(loaded.stack_demand_base);
            crate::uart::puts(", ");
            crate::uart::put_hex(loaded.stack_demand_top);
            crate::uart::puts(")\r\n");
            // Clear GP registers x0–x30, then set AArch64 SysV ABI entry args:
            //   x0 = argc, x1 = argv VA, x2 = envp VA.
            // x0 is written by dispatch() using this function's return value.
            // x1 and x2 are set here because dispatch() only forward-writes x0.
            // Reference: AArch64 SYSV ABI §3.4.1 — initial stack and register state.
            for reg in (*frame).x.iter_mut() {
                *reg = 0;
            }
            (*frame).x[1] = loaded.argv_va;
            (*frame).x[2] = loaded.envp_va;

            // Store the new environment in the process for getenv/putenv/execve.
            process.environ.clear();
            for env_entry in envp {
                if let Ok(s) = core::str::from_utf8(env_entry) {
                    process.environ.push(alloc::string::String::from(s));
                }
            }

            // Activate the new page table.
            if let Some(page_table) = &process.page_table {
                page_table.activate_el0();
            }
        }
    });

    // exec() does not return to userspace on success — ELR/SP were redirected.
    // dispatch() writes this return value into x0, which _start receives as argc.
    // Reference: AArch64 SYSV ABI — _start(x0=argc, x1=argv, x2=envp).
    loaded.argc as i64
}

// ---------------------------------------------------------------------------
// sys_getpid
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_getpid() -> i64 {
    // POSIX: getpid() returns the thread group ID (tgid), not the per-thread PID.
    // All threads in the same group return the same value.
    // Reference: Linux kernel/sys.c sys_getpid() — returns task->tgid.
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|process| process.tgid as i64)
            .unwrap_or(0)
    })
}

// ---------------------------------------------------------------------------
// sys_getppid
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_getppid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        match scheduler.current_process() {
            Some(process) => match process.parent_pid {
                Some(ppid) => ppid.index as i64,
                None => 0,
            },
            None => ESRCH,
        }
    })
}

// ---------------------------------------------------------------------------
// sys_clone — Phase 13 (POSIX threads)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Phase 13 — POSIX threads primitives
// ---------------------------------------------------------------------------

/// Linux-compatible clone flag: share virtual memory (used for thread creation).
const CLONE_VM: u64 = 0x0000_0100;

/// Linux-compatible clone flag: thread group membership.
const CLONE_THREAD: u64 = 0x0001_0000;

/// Linux-compatible clone flag: set TLS base from arg2.
const CLONE_SETTLS: u64 = 0x0008_0000;

/// Linux-compatible clone flag: write child TID to *ptid in parent.
/// Reference: Linux clone(2) man page, CLONE_PARENT_SETTID.
const CLONE_PARENT_SETTID: u64 = 0x0010_0000;

/// Linux-compatible clone flag: clear *ctid in child on thread exit (futex wake).
/// Reference: Linux clone(2) man page, CLONE_CHILD_CLEARTID.
const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;

// ---------------------------------------------------------------------------
// sys_clone — create a new thread or process
// ---------------------------------------------------------------------------

/// `clone(flags, child_stack, ptid, tls, ctid) → child_tid | -errno`
///
/// Implements a subset of Linux clone(2):
///   - `CLONE_VM | CLONE_THREAD`: create a new thread sharing the page table.
///   - Neither flag set: fall back to fork semantics.
///   - `CLONE_PARENT_SETTID`: write child TID to *ptid before returning.
///   - `CLONE_CHILD_CLEARTID`: store ctid in child's clear_child_tid field
///     so the kernel clears it and wakes a futex when the thread exits.
///
/// # Safety
/// `frame` must be the current process's exception frame on the kernel stack.
pub(super) unsafe fn sys_clone(
    frame: *mut ExceptionFrame,
    flags: u64,
    child_stack: u64,
    ptid: u64,
    tls: u64,
    ctid: u64,
) -> i64 {
    let is_thread_clone = (flags & CLONE_VM) != 0 && (flags & CLONE_THREAD) != 0;

    if !is_thread_clone {
        // Non-thread clone: fall back to fork().
        // SAFETY: frame is valid; IRQs are disabled at syscall entry.
        return match crate::scheduler::with_scheduler(|scheduler| scheduler.fork(frame)) {
            Ok(child_pid) => child_pid.index as i64,
            Err(_) => ENOMEM,
        };
    }

    // Thread clone: share address space with parent.
    // SAFETY: frame is valid; IRQs are disabled; clone_thread does not alias.
    let tls_requested = if (flags & CLONE_SETTLS) != 0 { tls } else { 0 };

    let child_pid_result = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.clone_thread(frame, child_stack, tls_requested)
    });

    let child_pid = match child_pid_result {
        Ok(pid) => pid,
        Err(_) => return ENOMEM,
    };

    // Write child TID to *ptid if CLONE_PARENT_SETTID is set.
    // The write happens in the parent before returning to either thread.
    // Reference: Linux clone(2) — CLONE_PARENT_SETTID writes before returning.
    if (flags & CLONE_PARENT_SETTID) != 0
        && ptid != 0
        && validate_user_pointer(ptid, core::mem::size_of::<u32>())
    {
        *(ptid as *mut u32) = child_pid.index as u32;
    }

    // Store ctid in child process for CLONE_CHILD_CLEARTID (futex-on-thread-exit).
    // Reference: Linux clone(2) — CLONE_CHILD_CLEARTID stores addr; set_tid_address
    // also stores it. The kernel zeros *ctid and wakes a futex on thread exit.
    if (flags & CLONE_CHILD_CLEARTID) != 0 && ctid != 0 {
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(child) = scheduler.process_mut(child_pid) {
                child.clear_child_tid = ctid;
            }
        });
    }

    child_pid.index as i64
}

// ---------------------------------------------------------------------------
// sys_gettid
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_gettid() -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_pid().index as i64
    })
}
