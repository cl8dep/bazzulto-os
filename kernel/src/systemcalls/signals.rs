// signals.rs — Signal handling syscall implementations.
//
// Syscalls: deliver_pending_signals, kill, sigaction, sigreturn, sigaltstack,
//           sigprocmask, sigpending, sigsuspend

use super::*;

// ---------------------------------------------------------------------------
// Signal delivery before returning to user space
// ---------------------------------------------------------------------------

pub(super) unsafe fn deliver_pending_signals(frame: *mut ExceptionFrame) {
    crate::scheduler::with_scheduler(|scheduler| {
        let current_pid = scheduler.current_pid();
        let signal_number = match scheduler.current_process() {
            Some(process) => process.take_pending_signal(),
            None => return,
        };

        let signal_number = match signal_number {
            Some(s) => s,
            None => return,
        };

        // DEBUG: log signal delivery.
        crate::drivers::uart::puts("[signal] delivering signum=");
        crate::drivers::uart::put_hex(signal_number as u64);
        crate::drivers::uart::puts(" to pid=");
        crate::drivers::uart::put_hex(current_pid.index as u64);
        crate::drivers::uart::puts("\r\n");

        // Look up the signal handler.
        let (action, trampoline_va) = match scheduler.current_process() {
            Some(process) => (
                process.signal_handlers[signal_number as usize],
                process.signal_trampoline_va(),
            ),
            None => return,
        };

        use crate::process::SignalAction;
        match action {
            SignalAction::Ignore => {}
            SignalAction::Default => {
                // Default action for most signals: terminate the process.
                // SIGCHLD and SIGURG: ignore by default.
                match signal_number {
                    17 | 23 => {} // SIGCHLD, SIGURG — ignore
                    _ => {
                        // Exit the process with signal number as exit code.
                        scheduler.exit(-(signal_number as i32));
                    }
                }
            }
            SignalAction::Handler { va: handler_va, on_stack } => {
                // Determine the stack pointer to use for signal delivery.
                //
                // If the handler was registered with SA_ONSTACK, the process has
                // an alternate stack configured, and we are not already executing
                // on that stack, switch to the top of the alternate stack.
                // Otherwise deliver on the current user stack.
                //
                // Reference: POSIX.1-2017 sigaltstack(2), sigaction(2) SA_ONSTACK.
                let base_sp = if on_stack {
                    // Read signal_stack and on_signal_stack state within the
                    // with_scheduler borrow already held above — we are already
                    // inside with_scheduler here, so access process fields directly
                    // by re-fetching from the same scheduler reference.
                    // NOTE: we are already inside `with_scheduler` in this closure.
                    // Accessing `process` again requires re-matching from `scheduler`.
                    // We do that by reading the fields we need before the match.
                    // The fields were already fetched via `action` above; here we
                    // need `signal_stack` and `on_signal_stack`.  Since we are in
                    // the same closure, fetch them inline.
                    //
                    // We can't call with_scheduler recursively, so read from the
                    // outer scheduler variable — but this closure already captures it.
                    // The outer with_scheduler closure uses a different binding below;
                    // we pull the process again from `scheduler` already in scope.
                    //
                    // The entire deliver_pending_signals is called from a single
                    // with_scheduler closure (line 437). We have `scheduler` in scope.
                    let use_alt = match scheduler.current_process() {
                        Some(p) => p.signal_stack.is_some() && !p.on_signal_stack,
                        None => false,
                    };
                    if use_alt {
                        if let Some(p) = scheduler.current_process_mut() {
                            p.on_signal_stack = true;
                            let ss = p.signal_stack.unwrap();
                            ss.base + ss.size as u64
                        } else {
                            (*frame).sp
                        }
                    } else {
                        (*frame).sp
                    }
                } else {
                    (*frame).sp
                };

                // Signal frame layout (grows downward from base_sp):
                //
                //   [base_sp - 32]  saved ELR_EL0  (pre-signal PC)
                //   [base_sp - 24]  saved SPSR_EL1 (pre-signal pstate)
                //   [base_sp - 16]  (alignment padding)
                //   [base_sp -  8]  (alignment padding)
                //   ← new sp (= base_sp - 32)
                //
                // sys_sigreturn restores ELR and SPSR from [sp+0] and [sp+8],
                // then advances sp by 32.
                //
                // Reference: ARM AAPCS64 §6.2 (stack must be 16-byte aligned at call).
                let new_sp = base_sp.wrapping_sub(32);
                core::ptr::write(new_sp as *mut u64, (*frame).elr);
                core::ptr::write((new_sp + 8) as *mut u64, (*frame).spsr);

                (*frame).sp = new_sp;
                // x0 = signal number (first argument to the handler).
                (*frame).x[0] = signal_number as u64;
                // x30 (link register) = signal trampoline.
                // The trampoline executes `svc #SIGRETURN` so that returning
                // from the handler lands in sys_sigreturn.
                (*frame).x[30] = trampoline_va;
                // ELR = handler entry point — eret resumes here.
                (*frame).elr = handler_va;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// sys_exit
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sys_kill — send a signal to a process
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_kill(target_pid: i32, signal_number: i32) -> i64 {
    if target_pid <= 0 {
        // POSIX.1-2017 kill(2): pid == 0 sends to the process group (not
        // supported yet); pid < 0 sends to a group (not supported yet).
        // ESRCH would mean "process not found" — incorrect here.
        // EPERM is the closest appropriate error for "not permitted / not
        // implemented" at this scope level.
        return EPERM;
    }
    if signal_number < 0 || signal_number as usize >= crate::process::SIGNAL_COUNT {
        return EINVAL;
    }
    let pid = crate::process::Pid::new(target_pid as u16, 1);
    crate::scheduler::with_scheduler(|scheduler| {
        if scheduler.send_signal_to(pid, signal_number as u8) {
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigaction — register a signal handler
// ---------------------------------------------------------------------------

/// Linux struct sigaction layout (AArch64):
///   offset  0: u64 sa_handler  (SIG_DFL=0, SIG_IGN=1, or function pointer)
///   offset  8: u64 sa_flags
///   offset 16: u64 sa_restorer (ignored — kernel uses its own trampoline)
///   offset 24: u64[16] sa_mask (128 bytes of blocked signal set during handler)
/// Total: 152 bytes.  We only read/write the first 24 bytes (handler + flags).
///
/// Reference: Linux include/uapi/asm-generic/signal.h, sigaction(2).
pub(super) unsafe fn sys_sigaction(
    signal_number: i32,
    new_act_ptr: u64,
    old_act_ptr: u64,
    _sigsetsize: usize,
) -> i64 {
    if signal_number <= 0 || signal_number as usize >= crate::process::SIGNAL_COUNT {
        return EINVAL;
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            // Write old action to *old_act_ptr if requested (24 bytes minimum).
            if old_act_ptr != 0 && validate_user_pointer(old_act_ptr, 24) {
                let out_ptr = old_act_ptr as *mut u64;
                let (handler_va, sa_flags_out) = match process.signal_handlers[signal_number as usize] {
                    crate::process::SignalAction::Handler { va, on_stack } => {
                        let flags = if on_stack {
                            crate::process::SA_ONSTACK as u64
                        } else {
                            0u64
                        };
                        (va, flags)
                    }
                    // SIG_IGN = 1 per Linux ABI.
                    crate::process::SignalAction::Ignore  => (1u64, 0u64),
                    // SIG_DFL = 0 per Linux ABI.
                    crate::process::SignalAction::Default => (0u64, 0u64),
                };
                *out_ptr         = handler_va;    // sa_handler at offset 0
                *out_ptr.add(1)  = sa_flags_out;  // sa_flags at offset 8
                *out_ptr.add(2)  = 0;             // sa_restorer at offset 16
            }

            // Install new action if new_act_ptr is non-null (read 16 bytes: handler + flags).
            if new_act_ptr != 0 && validate_user_pointer(new_act_ptr, 16) {
                let in_ptr = new_act_ptr as *const u64;
                let handler_va = *in_ptr;           // sa_handler at offset 0
                let sa_flags   = *in_ptr.add(1);    // sa_flags at offset 8

                let on_stack = (sa_flags & crate::process::SA_ONSTACK as u64) != 0;
                // SIG_DFL = 0, SIG_IGN = 1 per Linux ABI.
                let action = match handler_va {
                    0 => crate::process::SignalAction::Default,
                    1 => crate::process::SignalAction::Ignore,
                    va => crate::process::SignalAction::Handler { va, on_stack },
                };
                match process.set_signal_handler(signal_number as u8, action) {
                    Ok(()) => {}
                    Err(_) => return EINVAL,
                }
            }
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_getppid

// ---------------------------------------------------------------------------
// sys_sigreturn — restore context after signal handler
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_sigreturn(frame: *mut ExceptionFrame) -> i64 {
    // Restore pre-signal CPU state from the signal frame written by
    // deliver_pending_signals().
    //
    // Signal frame layout at sp (set up by deliver_pending_signals):
    //   [sp + 0]  saved ELR_EL0  (pre-signal PC)
    //   [sp + 8]  saved SPSR_EL1 (pre-signal pstate)
    //
    // After restoring, advance sp by 32 to discard the frame.
    //
    // Reference: POSIX.1-2017 sigreturn(2) — restore pre-signal context.
    let sp = (*frame).sp;
    (*frame).elr  = core::ptr::read(sp as *const u64);
    (*frame).spsr = core::ptr::read((sp + 8) as *const u64);
    (*frame).sp   = sp.wrapping_add(32);

    // If the process was executing on the alternate signal stack, mark that
    // we have returned from it so the next signal can use it again.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.on_signal_stack = false;
        }
    });

    // Return value of sigreturn is not observed — the restored ELR redirects
    // execution back to the pre-signal instruction.
    0
}

// ---------------------------------------------------------------------------
// sys_sigaltstack — set / query per-process alternate signal stack
// ---------------------------------------------------------------------------

/// Kernel-visible layout of the `stack_t` / `sigaltstack` struct.
///
/// Matches the layout expected by POSIX `sigaltstack(2)`:
///   Offset 0: ss_sp   (u64) — base pointer of the alternate stack region
///   Offset 8: ss_flags (u32) — SS_DISABLE (4) = disabled, SS_ONSTACK (1) = in use
///   Offset 16: ss_size (u64) — size of the region in bytes
///
/// Reference: POSIX.1-2017 `sys/signal.h`, Linux `asm/signal.h`.
#[repr(C)]
struct UserSignalStack {
    ss_sp:    u64,
    ss_flags: u32,
    _pad:     u32,
    ss_size:  u64,
}

/// `sigaltstack(new_stack_ptr, old_stack_ptr) → 0 | -errno`
///
/// If `new_stack_ptr` is non-null: install the described alternate stack.
/// If `old_stack_ptr` is non-null: write the current alternate stack state.
/// Either pointer may be null (query-only or set-only).
///
/// Returns `EPERM` if called while executing on the alternate stack (`SS_ONSTACK`).
/// Returns `EINVAL` if the new stack size is smaller than the POSIX minimum (2048 bytes).
///
/// Reference: POSIX.1-2017 `sigaltstack(2)`.
pub(super) unsafe fn sys_sigaltstack(new_stack_ptr: u64, old_stack_ptr: u64) -> i64 {
    use crate::process::{SignalStack, SS_DISABLE, SS_ONSTACK};

    // Reject pointers outside user address space.
    let new_ptr_valid = new_stack_ptr != 0
        && new_stack_ptr < crate::process::USER_ADDR_LIMIT;
    let old_ptr_valid = old_stack_ptr != 0
        && old_stack_ptr < crate::process::USER_ADDR_LIMIT;

    crate::scheduler::with_scheduler(|scheduler| {
        let process = match scheduler.current_process_mut() {
            Some(p) => p,
            None => return ESRCH,
        };

        // Cannot change the alternate stack while executing on it.
        if new_ptr_valid && process.on_signal_stack {
            return -(crate::process::SS_ONSTACK as i64); // EPERM-like; conventionally EINVAL on Linux
        }

        // Write the old stack state if requested.
        if old_ptr_valid {
            let out = &mut *(old_stack_ptr as *mut UserSignalStack);
            match process.signal_stack {
                Some(ss) => {
                    out.ss_sp    = ss.base;
                    out.ss_flags = if process.on_signal_stack { SS_ONSTACK } else { 0 };
                    out.ss_size  = ss.size as u64;
                }
                None => {
                    out.ss_sp    = 0;
                    out.ss_flags = SS_DISABLE;
                    out.ss_size  = 0;
                }
            }
        }

        // Install the new stack if requested.
        if new_ptr_valid {
            let input = &*(new_stack_ptr as *const UserSignalStack);
            if input.ss_flags & SS_DISABLE != 0 {
                process.signal_stack = None;
            } else {
                // POSIX minimum alternate stack size: MINSIGSTKSZ = 2048 bytes.
                // Reference: POSIX.1-2017 `<signal.h>`.
                const MINSIGSTKSZ: usize = 2048;
                if (input.ss_size as usize) < MINSIGSTKSZ {
                    return EINVAL;
                }
                process.signal_stack = Some(SignalStack {
                    base:  input.ss_sp,
                    size:  input.ss_size as usize,
                    flags: 0,
                });
            }
        }

        0
    })
}

// ---------------------------------------------------------------------------
// Phase 7 — Scheduler: nice, rlimits, process groups, sessions


/// Bits for signals that can never be blocked (SIGKILL=9, SIGSTOP=19).
const UNBLOCKABLE_SIGNALS_MASK: u64 = (1u64 << 9) | (1u64 << 19);

pub(super) unsafe fn sys_sigprocmask(how: i32, set_ptr: *const u64, old_set_ptr: *mut u64, _sigsetsize: usize) -> i64 {
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            // Return old mask if requested.
            if !old_set_ptr.is_null()
                && (old_set_ptr as u64) < crate::process::USER_ADDR_LIMIT
            {
                *old_set_ptr = process.signal_mask;
            }

            // Apply new mask if provided.
            if !set_ptr.is_null()
                && (set_ptr as u64) < crate::process::USER_ADDR_LIMIT
            {
                let new_set = *set_ptr & !UNBLOCKABLE_SIGNALS_MASK;
                process.signal_mask = match how {
                    SIG_BLOCK   => process.signal_mask | new_set,
                    SIG_UNBLOCK => process.signal_mask & !new_set,
                    SIG_SETMASK => new_set,
                    _           => return EINVAL,
                };
            }
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigpending — return set of pending signals
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_sigpending(set_ptr: *mut u64, _sigsetsize: usize) -> i64 {
    if set_ptr.is_null() || (set_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    }

    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process() {
            *set_ptr = process.pending_signals.load(core::sync::atomic::Ordering::Acquire)
                & process.signal_mask;
            0
        } else {
            ESRCH
        }
    })
}

// ---------------------------------------------------------------------------
// sys_sigsuspend — replace signal mask and suspend until signal arrives
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_sigsuspend(frame: *mut ExceptionFrame, mask_ptr: *const u64, _sigsetsize: usize) -> i64 {
    // Dereference the user pointer to get the signal mask.
    // Reference: POSIX.1-2017 sigsuspend(2) — mask is a pointer to sigset_t.
    let mask = if mask_ptr.is_null() || (mask_ptr as u64) >= crate::process::USER_ADDR_LIMIT {
        return EINVAL;
    } else {
        *mask_ptr
    };

    // Install the new mask (clearing unblockable bits).
    let applied_mask = mask & !UNBLOCKABLE_SIGNALS_MASK;
    let old_mask = crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            let old = process.signal_mask;
            process.signal_mask = applied_mask;
            old
        } else {
            0
        }
    });

    // Block until a non-masked signal is pending.
    loop {
        // Check for deliverable signal.
        let has_signal = crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process() {
                let pending = process.pending_signals.load(core::sync::atomic::Ordering::Acquire);
                (pending & !process.signal_mask) != 0
            } else {
                true // exit loop if process gone
            }
        });

        if has_signal {
            break;
        }

        // No deliverable signal — yield.
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.schedule();
        });
    }

    // Restore old mask.
    crate::scheduler::with_scheduler(|scheduler| {
        if let Some(process) = scheduler.current_process_mut() {
            process.signal_mask = old_mask;
        }
    });

    // Deliver any pending signals before returning.
    deliver_pending_signals(frame);

    // sigsuspend always returns -EINTR.
    EINTR
}

