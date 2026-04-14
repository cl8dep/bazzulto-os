// threads.rs — Threading primitives and epoll syscall implementations.
//
// Syscalls: futex, epoll_create1, epoll_ctl, epoll_wait, set_tls, framebuffer_map

use super::*;

// EpollInstanceTable — global registry keyed by inode number
// ---------------------------------------------------------------------------
//
// We cannot downcast `Arc<dyn Inode>` to `Arc<EpollInstance>` without the
// `Any` trait, which is not available in our no_std / no-core-introspection
// environment.  We maintain a parallel global table: `sys_epoll_create1`
// registers the new instance here; `sys_epoll_ctl` and `sys_epoll_wait` look
// it up by the inode number stored in the FileDescriptor.

use alloc::sync::Arc;

struct EpollInstanceTableInner {
    entries: alloc::vec::Vec<(u64, Arc<crate::fs::epoll::EpollInstance>)>,
}

struct EpollInstanceTable(core::cell::UnsafeCell<EpollInstanceTableInner>);

// SAFETY: single-core kernel; all kernel code runs with IRQs disabled.
unsafe impl Sync for EpollInstanceTable {}

static EPOLL_INSTANCE_TABLE: EpollInstanceTable = EpollInstanceTable(
    core::cell::UnsafeCell::new(EpollInstanceTableInner {
        entries: alloc::vec::Vec::new(),
    })
);

impl EpollInstanceTable {
    /// Register an EpollInstance under its inode number.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    unsafe fn register(
        &self,
        inode_number: u64,
        instance: Arc<crate::fs::epoll::EpollInstance>,
    ) {
        let inner = &mut *self.0.get();
        inner.entries.push((inode_number, instance));
    }

    /// Call `function` with the EpollInstance matching `inode_number`.
    ///
    /// Returns `Some(result)` if found, `None` if not found.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    unsafe fn get_and_call<F, R>(&self, inode_number: u64, function: F) -> Option<R>
    where
        F: FnOnce(&crate::fs::epoll::EpollInstance) -> R,
    {
        let inner = &*self.0.get();
        for (key, instance) in &inner.entries {
            if *key == inode_number {
                return Some(function(instance));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_futex — fast user-space mutex wait/wake
// ---------------------------------------------------------------------------

/// `futex(uaddr, op, val, timeout_ptr)` — minimum implementation for
/// `pthread_mutex_lock` / `pthread_mutex_unlock`.
///
/// Operations:
///   FUTEX_WAIT (0): if `*uaddr == val`, sleep on `uaddr`.
///                   Returns 0 on wakeup, -EAGAIN if `*uaddr != val`.
///   FUTEX_WAKE (1): wake up to `val` processes sleeping on `uaddr`.
///                   Returns the number of processes woken.
///
/// `timeout_ptr` is accepted but ignored (indefinite sleep); upgrade later.
///
/// Reference: Linux `futex(2)`, `kernel/futex/core.c`.
///
/// # Safety
/// Must be called with IRQs disabled (scheduler invariant).
pub(super) unsafe fn sys_futex(
    uaddr: u64,
    operation: i32,
    value: u32,
    _timeout_ptr: u64,
) -> i64 {
    if !validate_user_pointer(uaddr, core::mem::size_of::<u32>()) {
        return EINVAL;
    }

    match operation & 0x7F { // mask off FUTEX_PRIVATE_FLAG (0x80)
        FUTEX_WAIT => {
            // Read the current value at uaddr.
            let current_value = *(uaddr as *const u32);
            if current_value != value {
                // Value changed before we could sleep — report and let caller retry.
                return EAGAIN;
            }

            // Enqueue the current PID in the wait queue for this address.
            let current_pid = crate::scheduler::with_scheduler(|scheduler| {
                scheduler.current_pid()
            });

            {
                let table = &mut *FUTEX_TABLE.0.get();
                table.entry(uaddr).or_insert_with(VecDeque::new).push_back(current_pid);
            }

            // Sleep indefinitely; FUTEX_WAKE will transition us to Ready.
            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process_mut() {
                    process.state = crate::process::ProcessState::Sleeping {
                        wake_at_tick: u64::MAX,
                    };
                }
                scheduler.schedule();
            });

            0
        }

        FUTEX_WAKE => {
            // Wake up to `value` waiters for this address.
            let wake_count = value as usize;
            let mut woken: usize = 0;

            let pids_to_wake: alloc::vec::Vec<Pid> = {
                let table = &mut *FUTEX_TABLE.0.get();
                match table.get_mut(&uaddr) {
                    Some(queue) => {
                        let n = wake_count.min(queue.len());
                        queue.drain(..n).collect()
                    }
                    None => alloc::vec::Vec::new(),
                }
            };

            crate::scheduler::with_scheduler(|scheduler| {
                for pid in &pids_to_wake {
                    scheduler.futex_make_ready(*pid);
                    woken += 1;
                }
            });

            woken as i64
        }

        _ => EINVAL,
    }
}

/// `tcgetattr(fd, termios_ptr)` — get terminal attributes.
///
/// ABI: termios_ptr points to a `struct termios` (c_iflag, c_oflag, c_cflag, c_lflag: u32; c_cc: [u8; 32])
/// Total size: 4*4 + 32 = 48 bytes.
///
/// Reference: POSIX.1-2017 §11.1.

pub(super) unsafe fn sys_epoll_create1(_flags: i32) -> i64 {
    let epoll_instance = crate::fs::epoll::EpollInstance::new();
    let inode_number = crate::fs::inode::Inode::stat(&*epoll_instance).inode_number;
    // Register in the global table so epoll_ctl / epoll_wait can find it.
    // SAFETY: single-core, IRQs disabled.
    EPOLL_INSTANCE_TABLE.register(inode_number, epoll_instance.clone());
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let fd_table_arc = match fd_table_arc {
        Some(arc) => arc,
        None => return ESRCH,
    };
    let mut guard = fd_table_arc.lock();
    let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
        inode: epoll_instance,
        position: 0,
    };
    let new_fd = guard.install(descriptor);
    if new_fd < 0 { EMFILE as i64 } else { new_fd as i64 }
}

/// `epoll_ctl(epfd, op, fd, event_ptr)` — add/modify/remove interest.
///
/// `op` is one of EPOLL_CTL_ADD (1), EPOLL_CTL_DEL (2), EPOLL_CTL_MOD (3).
/// `event_ptr` points to a user-space `struct epoll_event` (events: u32, data: u64).
///
/// Returns 0 on success or a negative errno.
///
/// # Safety
/// Must be called with IRQs disabled.
pub(super) unsafe fn sys_epoll_ctl(epfd: i32, operation: i32, watched_fd: i32, event_ptr: u64) -> i64 {
    use crate::fs::epoll::{EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};

    if epfd < 0 || watched_fd < 0 {
        return EBADF;
    }

    // DEL does not require a valid event_ptr.
    let (event_mask, user_data) = if operation == EPOLL_CTL_DEL {
        (0u32, 0u64)
    } else {
        if !validate_user_pointer(event_ptr, core::mem::size_of::<crate::fs::epoll::EpollEvent>()) {
            return EINVAL;
        }
        // SAFETY: event_ptr is validated above.
        let raw_event = core::ptr::read_unaligned(
            event_ptr as *const crate::fs::epoll::EpollEvent,
        );
        (raw_event.events, raw_event.data)
    };

    // Obtain the inode_number of the epoll fd.
    let inode_number_option: Option<u64> = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            match guard.get(epfd as usize)? {
                crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                    Some(inode.stat().inode_number)
                }
                _ => None,
            }
        })
    };

    let inode_number = match inode_number_option {
        Some(n) => n,
        None => return EBADF,
    };

    EPOLL_INSTANCE_TABLE.get_and_call(inode_number, |instance| {
        match operation {
            EPOLL_CTL_ADD => instance.ctl_add(watched_fd, event_mask, user_data),
            EPOLL_CTL_DEL => instance.ctl_del(watched_fd),
            EPOLL_CTL_MOD => instance.ctl_mod(watched_fd, event_mask, user_data),
            _ => EINVAL,
        }
    }).unwrap_or(EBADF)
}

/// `epoll_wait(epfd, events_ptr, maxevents, timeout_ms)` — wait for events.
///
/// Writes up to `maxevents` ready `EpollEvent` records to `events_ptr`.
/// Returns the number of events written, 0 on timeout, or a negative errno.
///
/// Blocking behaviour:
///   timeout_ms == 0:  return immediately (non-blocking check).
///   timeout_ms  > 0:  yield once per timer tick until deadline or event.
///   timeout_ms == -1: block indefinitely until at least one event is ready.
///
/// Reference: Linux `fs/eventpoll.c` `ep_poll()`.
///
/// # Safety
/// Must be called with IRQs disabled.
pub(super) unsafe fn sys_epoll_wait(
    epfd: i32,
    events_ptr: u64,
    maxevents: i32,
    timeout_ms: i32,
) -> i64 {
    if epfd < 0 {
        return EBADF;
    }
    if maxevents <= 0 {
        return EINVAL;
    }
    let max = maxevents as usize;
    let event_size = core::mem::size_of::<crate::fs::epoll::EpollEvent>();
    if !validate_user_pointer(events_ptr, max * event_size) {
        return EINVAL;
    }

    // Identify the epoll instance by its inode number.
    let inode_number_option: Option<u64> = {
        let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process()
                .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
        });
        fd_table_arc.and_then(|arc| {
            let guard = arc.lock();
            match guard.get(epfd as usize)? {
                crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                    Some(inode.stat().inode_number)
                }
                _ => None,
            }
        })
    };

    let inode_number = match inode_number_option {
        Some(n) => n,
        None => return EBADF,
    };

    // Calculate the deadline tick (for timeout_ms > 0).
    let deadline_tick: Option<u64> = if timeout_ms > 0 {
        let tick_interval_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
        let ticks_needed = ((timeout_ms as u64) + tick_interval_ms - 1) / tick_interval_ms;
        Some(crate::platform::qemu_virt::timer::current_tick()
            .saturating_add(ticks_needed))
    } else {
        None
    };

    // Temporary event buffer on the stack (max 64 events per call).
    // EpollEvent is Copy, so we can use a simple array initialiser.
    let mut event_buffer = [crate::fs::epoll::EpollEvent { events: 0, data: 0 }; 64];

    loop {
        let effective_max = max.min(event_buffer.len());

        let ready_count = EPOLL_INSTANCE_TABLE.get_and_call(
            inode_number,
            // SAFETY: single-core, IRQs disabled; slice length bounded by effective_max.
            |instance| unsafe {
                instance.collect_ready_events(&mut event_buffer[..effective_max], effective_max)
            },
        ).unwrap_or(0);

        if ready_count > 0 {
            // Copy ready events to the user buffer.
            let output_ptr = events_ptr as *mut crate::fs::epoll::EpollEvent;
            for event_index in 0..ready_count {
                // SAFETY: events_ptr is validated; index within bounds.
                core::ptr::write_unaligned(
                    output_ptr.add(event_index),
                    core::ptr::read_unaligned(&event_buffer[event_index]),
                );
            }
            return ready_count as i64;
        }

        if timeout_ms == 0 {
            return 0; // non-blocking — no events ready
        }
        if let Some(deadline) = deadline_tick {
            if crate::platform::qemu_virt::timer::current_tick() >= deadline {
                return 0; // timed out
            }
        }
        // Yield to allow other processes to run and (possibly) produce events.
        crate::scheduler::with_scheduler(|scheduler| { scheduler.schedule(); });
    }
}

// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_set_tls(tls_base: u64) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.tls_base = tls_base;
        }
    });
    // Write the new TLS base into the hardware register immediately so the
    // calling thread sees it without waiting for a context switch.
    //
    // Reference: ARM ARM DDI 0487 D13.2.116 "TPIDR_EL0".
    // SAFETY: TPIDR_EL0 is a user-accessible opaque register; writing it is
    // safe from EL1 at any time.
    core::arch::asm!(
        "msr tpidr_el0, {tls}",
        tls = in(reg) tls_base,
        options(nostack, nomem),
    );
    0
}

// ---------------------------------------------------------------------------
// sys_gettid — return the calling thread's TID
// ---------------------------------------------------------------------------

/// `gettid() → tid`
///
/// Returns the current thread's own TID (= `pid.index`), not the TGID.
/// Aligns with Linux: `gettid()` returns the per-thread ID; `getpid()` returns
/// the thread group leader's PID (tgid).
///
/// # Safety
/// No preconditions beyond the standard syscall invariant.

pub(super) unsafe fn sys_framebuffer_map(out: *mut u64) -> i64 {
    if !validate_user_pointer(out as u64, 8 * core::mem::size_of::<u64>()) {
        return EINVAL;
    }

    // Only a process with CAP_DISPLAY may map the framebuffer.
    let authorized = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.capabilities & crate::process::CAP_DISPLAY != 0)
            .unwrap_or(false)
    });
    if !authorized {
        return EPERM;
    }

    let info = match crate::display::get() {
        Some(info) => info,
        None => return EINVAL,
    };

    let page_size = crate::memory::physical::read_page_size();
    let pages = ((info.size_bytes + page_size - 1) / page_size) as usize;

    let mapped_va = crate::memory::with_physical_allocator(|phys| {
        crate::scheduler::with_scheduler(|scheduler| {
            match scheduler.map_physical_pages_for_current(
                info.phys_base,
                pages,
                page_size,
                phys,
            ) {
                Some(va) => va as i64,
                None => ENOMEM,
            }
        })
    });

    if mapped_va < 0 {
        return mapped_va;
    }

    // Write descriptor to user buffer.
    let out_slice = core::slice::from_raw_parts_mut(out, 8);
    out_slice[0] = mapped_va as u64;
    out_slice[1] = info.width;
    out_slice[2] = info.height;
    out_slice[3] = info.stride;
    out_slice[4] = info.bpp as u64;
    out_slice[5] = ((info.red_mask_size   as u64) << 8) | info.red_mask_shift   as u64;
    out_slice[6] = ((info.green_mask_size as u64) << 8) | info.green_mask_shift as u64;
    out_slice[7] = ((info.blue_mask_size  as u64) << 8) | info.blue_mask_shift  as u64;

    0
}
