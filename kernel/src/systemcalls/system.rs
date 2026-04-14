// system.rs — System information and control syscall implementations.
//
// Syscalls: setfgpid, disk_info, getrandom, uname, sysinfo, getrusage,
//           prctl, alarm, machine_reboot, machine_poweroff

use super::*;

// ---------------------------------------------------------------------------
// sys_setfgpid — set foreground process
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_setfgpid(pid_arg: i32) -> i64 {
    if pid_arg <= 0 {
        return EINVAL;
    }
    let pid = crate::process::Pid::new(pid_arg as u16, 1);
    crate::scheduler::with_scheduler(|scheduler| {
        // Clear foreground from all processes, set on target.
        for slot_index in 0..crate::scheduler::PID_MAX {
            if let Some(process) = scheduler.process_mut(crate::process::Pid::new(slot_index as u16, 1)) {
                process.is_foreground = false;
            }
        }
        if let Some(process) = scheduler.process_mut(pid) {
            process.is_foreground = true;
            0i64
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_disk_info — return disk capacity and FAT32 info
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_disk_info(buf_ptr: *mut u64) -> i64 {
    if buf_ptr.is_null() || (buf_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }
    let capacity = crate::hal::disk::capacity();
    *buf_ptr = capacity;
    0
}

// ---------------------------------------------------------------------------
// sys_getrandom — fill a user buffer with pseudo-random bytes
// ---------------------------------------------------------------------------

/// Fill `buf[0..len]` with bytes from the kernel entropy pool.
///
/// The entropy pool is seeded from CNTPCT_EL0 + CNTFRQ_EL0 + TTBR1_EL1.
/// This is NOT cryptographically secure (no hardware TRNG on cortex-a72),
/// but it is suitable for ASLR, salts, and non-security-critical randomness.
///
/// Flags argument is ignored (Linux getrandom flags: GRND_NONBLOCK, GRND_RANDOM).
///
/// Reference: Linux sys_getrandom (random.c), POSIX.1-2017 §getentropy.
pub(super) unsafe fn sys_getrandom(buf_ptr: *mut u8, len: usize, _flags: u32) -> i64 {
    if buf_ptr.is_null()
        || len == 0
        || (buf_ptr as u64) >= crate::process::USER_ADDR_LIMIT
        || (buf_ptr as u64).saturating_add(len as u64) > crate::process::USER_ADDR_LIMIT
    {
        return EINVAL;
    }

    let mut written = 0usize;

    while written < len {
        // Generate 8 bytes of entropy per iteration using the same mix as ASLR.
        let cntpct: u64;
        let cntfrq: u64;
        let ttbr1: u64;
        core::arch::asm!(
            "mrs {cntpct}, cntpct_el0",
            "mrs {cntfrq}, cntfrq_el0",
            "mrs {ttbr1}, ttbr1_el1",
            cntpct = out(reg) cntpct,
            cntfrq = out(reg) cntfrq,
            ttbr1  = out(reg) ttbr1,
            options(nostack, nomem)
        );

        static ENTROPY_COUNTER: core::sync::atomic::AtomicU64 =
            core::sync::atomic::AtomicU64::new(0xDEAD_BEEF_0000_0001);
        let counter = ENTROPY_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let mut state = cntpct
            ^ cntfrq.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (ttbr1 >> 12)
            ^ counter.wrapping_mul(0x6c62_272e_07bb_0142);

        // Xorshift64 mixing.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;

        // Copy up to 8 bytes from `state` into `buf`.
        let remaining = len - written;
        let chunk = if remaining >= 8 { 8 } else { remaining };
        let state_bytes = state.to_le_bytes();
        core::ptr::copy_nonoverlapping(state_bytes.as_ptr(), buf_ptr.add(written), chunk);
        written += chunk;
    }

    len as i64
}

// ---------------------------------------------------------------------------
// sys_sigreturn — restore context after signal handler

//   [260..325] machine
//   [325..390] domainname
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_uname(buf: *mut u8) -> i64 {
    if !validate_user_pointer(buf as u64, 390) {
        return EINVAL;
    }

    const FIELD_LEN: usize = 65;
    let fields: [&[u8]; 6] = [
        b"Bazzulto",
        b"bazzulto",
        b"0.1.0",
        b"Bazzulto 0.1.0 (AArch64)",
        b"aarch64",
        b"",
    ];

    for (i, field) in fields.iter().enumerate() {
        let dst = buf.add(i * FIELD_LEN);
        let copy_len = field.len().min(FIELD_LEN - 1);
        core::ptr::copy_nonoverlapping(field.as_ptr(), dst, copy_len);
        *dst.add(copy_len) = 0; // NUL terminate
    }

    0
}

// ---------------------------------------------------------------------------
// sys_sysinfo — return system statistics
//
// ABI: sysinfo(info_ptr) where info_ptr points to a struct of u64 fields:
//   [0]  uptime in seconds
//   [1]  total RAM in bytes
//   [2]  free RAM in bytes
//   [3]  number of processes
// (Simplified subset of Linux struct sysinfo)
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_sysinfo(info_ptr: *mut u64) -> i64 {
    if !validate_user_pointer(info_ptr as u64, 4 * 8) {
        return EINVAL;
    }

    let uptime_seconds = crate::platform::qemu_virt::timer::current_tick()
        * crate::platform::qemu_virt::timer::TICK_INTERVAL_MS
        / 1000;

    let (total_ram, free_ram) = crate::memory::physical_stats();

    let process_count = crate::scheduler::with_scheduler(|s| s.alive_process_count());

    *info_ptr          = uptime_seconds;
    *info_ptr.add(1)   = total_ram;
    *info_ptr.add(2)   = free_ram;
    *info_ptr.add(3)   = process_count as u64;

    0
}

pub(super) unsafe fn sys_getrusage(who: i32, usage_ptr: *mut u64) -> i64 {
    let _ = who;
    if !validate_user_pointer(usage_ptr as u64, 4 * 8) {
        return EINVAL;
    }

    let (user_ticks, sys_ticks) = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| (p.user_ticks, p.sys_time_ticks))
            .unwrap_or((0, 0))
    });

    // Convert ticks to (seconds, microseconds).
    // TICK_INTERVAL_MS is the timer period in ms; ticks_per_second = 1000 / interval.
    let ticks_per_second = 1000 / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;
    let user_seconds  = user_ticks / ticks_per_second;
    let user_useconds = (user_ticks % ticks_per_second)
        * (1_000_000 / ticks_per_second);
    // sys_time_ticks counts one tick per syscall entry.  Reuse the same
    // ticks_per_second denominator so both times are on the same scale.
    let sys_seconds   = sys_ticks / ticks_per_second;
    let sys_useconds  = (sys_ticks % ticks_per_second)
        * (1_000_000 / ticks_per_second);

    *usage_ptr          = user_seconds;  // ru_utime.tv_sec
    *usage_ptr.add(1)   = user_useconds; // ru_utime.tv_usec
    *usage_ptr.add(2)   = sys_seconds;   // ru_stime.tv_sec
    *usage_ptr.add(3)   = sys_useconds;  // ru_stime.tv_usec

    0
}

// ---------------------------------------------------------------------------
// sys_prctl — process control operations
//
// Only PR_SET_NAME (15) and PR_GET_NAME (16) are implemented.
// ---------------------------------------------------------------------------

const PR_SET_NAME: i32 = 15;
const PR_GET_NAME: i32 = 16;

pub(super) unsafe fn sys_prctl(option: i32, name_ptr: *const u8, name_len: usize) -> i64 {
    match option {
        PR_SET_NAME => {
            if !validate_user_pointer(name_ptr as u64, 1) {
                return EINVAL;
            }
            let copy_len = name_len.min(15);
            let mut name_buf = [0u8; 16];
            core::ptr::copy_nonoverlapping(name_ptr, name_buf.as_mut_ptr(), copy_len);
            name_buf[copy_len] = 0;

            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process_mut() {
                    process.name = name_buf;
                    0
                } else {
                    ESRCH
                }
            })
        }
        PR_GET_NAME => {
            if !validate_user_pointer(name_ptr as u64, 16) {
                return EINVAL;
            }
            crate::scheduler::with_scheduler(|scheduler| {
                if let Some(process) = scheduler.current_process() {
                    core::ptr::copy_nonoverlapping(
                        process.name.as_ptr(),
                        name_ptr as *mut u8,
                        16,
                    );
                    0
                } else {
                    ESRCH
                }
            })
        }
        _ => EINVAL,
    }
}

/// Schedule delivery of SIGALRM after `seconds` seconds.
///
/// Returns the number of seconds remaining on any previously scheduled alarm
/// (0 if none was set).  Passing `seconds == 0` cancels any pending alarm.
///
/// The alarm fires at most once: after delivery, `alarm_deadline_tick` is
/// reset to 0 and the process must call `alarm()` again if another one is needed.
///
/// Reference: POSIX.1-2017 `alarm(2)`.
pub(super) unsafe fn sys_alarm(seconds: u64) -> i64 {
    // TICK_INTERVAL_MS = 10 ms → 100 ticks per second.
    const TICKS_PER_SECOND: u64 =
        1_000 / crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;

    let now_tick = crate::platform::qemu_virt::timer::current_tick();

    crate::scheduler::with_scheduler(|scheduler| {
        let process = match scheduler.current_process_mut() {
            Some(p) => p,
            None => return 0i64,
        };

        // Compute remaining seconds on any existing alarm.
        let remaining_seconds = if process.alarm_deadline_tick != 0
            && process.alarm_deadline_tick > now_tick
        {
            let remaining_ticks = process.alarm_deadline_tick - now_tick;
            // Round up: partial ticks count as a full second.
            (remaining_ticks + TICKS_PER_SECOND - 1) / TICKS_PER_SECOND
        } else {
            0
        };

        if seconds == 0 {
            // Cancel any pending alarm.
            process.alarm_deadline_tick = 0;
        } else {
            process.alarm_deadline_tick = now_tick + seconds * TICKS_PER_SECOND;
        }

        remaining_seconds as i64
    })
}

// ---------------------------------------------------------------------------
// sys_machine_reboot / sys_machine_poweroff — PSCI machine control
// ---------------------------------------------------------------------------

/// Reboot the machine via PSCI SYSTEM_RESET.
///
/// Uses the PSCI v0.2 / v1.0 `SYSTEM_RESET` function (ID 0x84000009)
/// invoked as an HVC call.  QEMU virt exposes PSCI via HVC by default.
///
/// Reference: ARM DEN0022D — Power State Coordination Interface (PSCI) §5.16.
///
/// # Safety
/// Terminates all execution unconditionally.
pub(super) unsafe fn sys_machine_reboot() -> i64 {
    // PSCI_SYSTEM_RESET = 0x84000009 (SMC32 calling convention, function ID).
    // x0 = function ID; HVC #0.
    // MOVZ x0, #0x0009          → x0[15:0]  = 0x0009
    // MOVK x0, #0x8400, lsl #16 → x0[31:16] = 0x8400  → x0 = 0x84000009
    core::arch::asm!(
        "movz x0, #0x0009",
        "movk x0, #0x8400, lsl #16",
        "hvc #0",
        options(nostack, noreturn)
    );
}

/// Power off the machine via PSCI SYSTEM_OFF.
///
/// Uses the PSCI v0.2 / v1.0 `SYSTEM_OFF` function (ID 0x84000008)
/// invoked as an HVC call.
///
/// Reference: ARM DEN0022D — Power State Coordination Interface (PSCI) §5.15.
///
/// # Safety
/// Terminates all execution unconditionally.
pub(super) unsafe fn sys_machine_poweroff() -> i64 {
    // PSCI_SYSTEM_OFF = 0x84000008.
    // MOVZ x0, #0x0008          → x0[15:0]  = 0x0008
    // MOVK x0, #0x8400, lsl #16 → x0[31:16] = 0x8400  → x0 = 0x84000008
    core::arch::asm!(
        "movz x0, #0x0008",
        "movk x0, #0x8400, lsl #16",
        "hvc #0",
        options(nostack, noreturn)
    );
}

