// time.rs — Time-related syscall implementations.
//
// Syscalls: clock_gettime, nanosleep, gettimeofday

use super::*;

// ---------------------------------------------------------------------------
// sys_clock_gettime — return nanosecond timestamp from CNTPCT_EL0
// ---------------------------------------------------------------------------

/// POSIX CLOCK_REALTIME: wall-clock time (seconds since Unix epoch).
/// Value 0 matches both POSIX and Linux ABI.
const CLOCK_REALTIME: i32 = 0;

/// POSIX CLOCK_MONOTONIC: time since an unspecified point (here: boot).
/// Value 1 matches both POSIX and Linux ABI.
const CLOCK_MONOTONIC: i32 = 1;

pub(super) unsafe fn sys_clock_gettime(clock_id: i32, timespec_ptr: *mut u64) -> i64 {
    if clock_id != CLOCK_REALTIME && clock_id != CLOCK_MONOTONIC {
        return EINVAL;
    }

    // struct timespec { time_t tv_sec; long tv_nsec; } — two 8-byte words.
    let (seconds, nanoseconds) = match clock_id {
        CLOCK_MONOTONIC => {
            // Read hardware counter and frequency for nanosecond-precision monotonic time.
            let cntpct: u64;
            let cntfrq: u64;
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) cntpct, options(nostack, nomem));
            core::arch::asm!("mrs {}, cntfrq_el0", out(reg) cntfrq, options(nostack, nomem));
            (cntpct / cntfrq, (cntpct % cntfrq) * 1_000_000_000 / cntfrq)
        }
        CLOCK_REALTIME => {
            // Use PL031-derived wall clock: boot-time epoch + elapsed ticks.
            let tick = crate::platform::qemu_virt::timer::current_tick();
            let tick_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
            crate::platform::qemu_virt::rtc::realtime_now(tick, tick_ms)
        }
        _ => return EINVAL,
    };

    if let Err(e) = put_user(timespec_ptr, seconds) { return e; }
    if let Err(e) = put_user(timespec_ptr.add(1), nanoseconds) { return e; }
    0
}

// ---------------------------------------------------------------------------
// sys_nanosleep — put the calling process to sleep for the requested duration
// ---------------------------------------------------------------------------

/// `sys_nanosleep(req, rem)` — sleep for the duration specified by `*req`.
///
/// If a signal interrupts the sleep, writes the remaining time into `*rem`
/// (if `rem` is a valid user pointer) and returns `EINTR`.
///
/// Reference: POSIX.1-2017 nanosleep(2).
pub(super) unsafe fn sys_nanosleep(timespec_ptr: *const u64, rmtp: *mut u64) -> i64 {
    let seconds = match get_user(timespec_ptr) {
        Ok(v) => v,
        Err(_) => return EINVAL,
    };
    let nanoseconds = match get_user(timespec_ptr.add(1)) {
        Ok(v) => v,
        Err(_) => return EINVAL,
    };

    // Convert the requested duration to kernel ticks.
    // One tick = TICK_INTERVAL_MS milliseconds = TICK_INTERVAL_MS * 1_000_000 ns.
    // We add one extra tick to guarantee at least the requested duration elapses
    // (the current tick may be nearly expired when we read it).
    let tick_interval_ns: u64 =
        crate::platform::qemu_virt::timer::TICK_INTERVAL_MS * 1_000_000;
    let total_ns = seconds
        .saturating_mul(1_000_000_000)
        .saturating_add(nanoseconds);
    let ticks_to_sleep = total_ns / tick_interval_ns + 1;

    if ticks_to_sleep == 0 {
        // Zero-duration sleep — just yield.
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.schedule();
        });
        return 0;
    }

    let now_tick = crate::platform::qemu_virt::timer::current_tick();
    let wake_at_tick = now_tick.saturating_add(ticks_to_sleep);

    // Transition the current process to Sleeping, then yield.
    // The scheduler's wake-up logic (in schedule()) will move it back to
    // Ready once current_tick() >= wake_at_tick.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.state = crate::process::ProcessState::Sleeping { wake_at_tick };
        }
        // schedule() will see state != Running and will not re-enqueue us.
        scheduler.schedule();
    });

    // After waking, check whether a signal caused the early wake-up.
    // `pending_signals != 0` means at least one signal is queued; deliver_pending_signals
    // (called by dispatch() after this function returns) will handle it.
    // POSIX.1-2017 nanosleep(2): if interrupted by a signal, write remaining
    // time to *rmtp and return EINTR.
    let has_pending_signal = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| p.pending_signals.load(core::sync::atomic::Ordering::Acquire) != 0)
            .unwrap_or(false)
    });

    if has_pending_signal {
        let woke_at = crate::platform::qemu_virt::timer::current_tick();
        // Write remaining time into *rmtp if the pointer is valid.
        if !rmtp.is_null() {
            let remaining_ticks = wake_at_tick.saturating_sub(woke_at);
            let remaining_ns = remaining_ticks.saturating_mul(tick_interval_ns);
            let remaining_secs = remaining_ns / 1_000_000_000;
            let remaining_nsec = remaining_ns % 1_000_000_000;
            // Best-effort write; ignore EFAULT on rmtp (POSIX allows this).
            let _ = put_user(rmtp, remaining_secs);
            let _ = put_user(rmtp.add(1), remaining_nsec);
        }
        return EINTR;
    }

    0
}

// ---------------------------------------------------------------------------

// ABI: gettimeofday(tv_ptr, tz_ptr) where tv_ptr points to two u64 words.
// Returns wall-clock time via the PL031 RTC snapshot plus elapsed ticks.
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_gettimeofday(tv_ptr: *mut u64, tz_ptr: u64) -> i64 {
    if !tv_ptr.is_null() {
        let tick    = crate::platform::qemu_virt::timer::current_tick();
        let tick_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
        let (seconds, nanoseconds) =
            crate::platform::qemu_virt::rtc::realtime_now(tick, tick_ms);
        let useconds = nanoseconds / 1_000;

        if let Err(e) = put_user(tv_ptr, seconds) { return e; }
        if let Err(e) = put_user(tv_ptr.add(1), useconds) { return e; }
    }

    // tz_ptr: if non-null, write {tz_minuteswest=0, tz_dsttime=0} (UTC, no DST).
    // Reference: Linux gettimeofday(2) — timezone struct is deprecated; callers
    // should pass NULL. We zero-fill for compatibility.
    if tz_ptr != 0 {
        if let Err(e) = put_user(tz_ptr as *mut u64, 0u64) { return e; }
        if let Err(e) = put_user((tz_ptr + 8) as *mut u64, 0u64) { return e; }
    }

    0
}

// ---------------------------------------------------------------------------
// sys_poll — wait for events on file descriptors
