// systemcalls/bpm.rs — Binary Permission Model syscall implementations.
//
// Syscalls:
//   BPM_REGISTER (167):     permissiond registers itself with the kernel.
//   BPM_READ_REQUEST (168): permissiond reads next pending PermRequest.
//   BPM_RESPOND (169):      permissiond submits a decision for a blocked process.

use super::*;

/// sys_register_permissiond() → register the calling process as permissiond.
///
/// Only one process may register.  The caller must have the
/// `RegisterPermissiond` action permission (granted by bzinit at spawn time).
///
/// Returns 0 on success, negative errno on failure.
pub(super) unsafe fn sys_register_permissiond() -> i64 {
    let (pid, has_perm, action_count, is_tier1) = crate::scheduler::with_scheduler(|s| {
        let pid = s.current_pid();
        let (has_perm, action_count) = s.current_process().map(|p| {
            let has = p.granted_actions.contains(&crate::permission::ActionPermission::RegisterPermissiond);
            (has, p.granted_actions.len())
        }).unwrap_or((false, 0));
        // Check if this PID's binary path would be Tier 1.
        let is_tier1 = true; // can't easily check from here
        (pid, has_perm, action_count, is_tier1)
    });

    crate::drivers::uart::puts("[bpm-reg] pid=");
    crate::drivers::uart::put_hex(pid.index as u64);
    crate::drivers::uart::puts(" has_perm=");
    crate::drivers::uart::puts(if has_perm { "yes" } else { "no" });
    crate::drivers::uart::puts(" actions=");
    crate::drivers::uart::put_hex(action_count as u64);
    crate::drivers::uart::puts("\r\n");

    if !has_perm {
        return EPERM;
    }

    match crate::permission::ipc_channel::register_permissiond(pid) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// sys_bpm_read_request(buf_ptr, buf_len) → read the next pending PermRequest.
///
/// Only callable by the registered permissiond.
/// Writes a serialized PermRequest to the user buffer.
///
/// Format (binary, little-endian):
///   u32: blocked_pid_index
///   u32: caller_uid
///   u8:  has_elf_section (0 or 1)
///   u8:  has_tty (0 or 1)
///   u16: hash_hex_len
///   [u8; hash_hex_len]: hash_hex bytes
///   u16: binary_path_len
///   [u8; binary_path_len]: binary_path bytes
///
/// Returns bytes written on success, 0 if no pending requests, negative errno.
pub(super) unsafe fn sys_bpm_read_request(buf_ptr: *mut u8, buf_len: usize) -> i64 {
    if !validate_user_pointer(buf_ptr as u64, buf_len) {
        return EFAULT;
    }

    let caller_pid = crate::scheduler::with_scheduler(|s| s.current_pid());
    let request = match crate::permission::ipc_channel::read_next_request(caller_pid) {
        Some(req) => req,
        None => return 0, // No pending requests.
    };

    // Serialize into user buffer.
    let hash_bytes = request.hash_hex.as_bytes();
    let path_bytes = request.binary_path.as_bytes();
    let needed = 4 + 4 + 1 + 1 + 2 + hash_bytes.len() + 2 + path_bytes.len();
    if buf_len < needed {
        return EINVAL;
    }

    let buf = core::slice::from_raw_parts_mut(buf_ptr, buf_len);
    let mut off = 0;

    // blocked_pid_index (u32 LE)
    buf[off..off+4].copy_from_slice(&(request.blocked_pid.index as u32).to_le_bytes());
    off += 4;
    // caller_uid (u32 LE)
    buf[off..off+4].copy_from_slice(&request.caller_uid.to_le_bytes());
    off += 4;
    // has_elf_section (u8)
    buf[off] = if request.has_elf_section { 1 } else { 0 };
    off += 1;
    // has_tty (u8)
    buf[off] = if request.has_tty { 1 } else { 0 };
    off += 1;
    // hash_hex_len (u16 LE) + hash_hex bytes
    buf[off..off+2].copy_from_slice(&(hash_bytes.len() as u16).to_le_bytes());
    off += 2;
    buf[off..off+hash_bytes.len()].copy_from_slice(hash_bytes);
    off += hash_bytes.len();
    // binary_path_len (u16 LE) + binary_path bytes
    buf[off..off+2].copy_from_slice(&(path_bytes.len() as u16).to_le_bytes());
    off += 2;
    buf[off..off+path_bytes.len()].copy_from_slice(path_bytes);
    off += path_bytes.len();

    off as i64
}

/// sys_bpm_respond(blocked_pid_index, decision, patterns_ptr, patterns_len)
///
/// Only callable by the registered permissiond.
///
/// decision:
///   0 = Denied
///   1 = Granted (patterns_ptr contains comma-separated path patterns)
///   2 = GrantedInherited (use parent's permissions)
///
/// Returns 0 on success, negative errno on failure.
pub(super) unsafe fn sys_bpm_respond(
    blocked_pid_index: u32,
    decision: u32,
    patterns_ptr: *const u8,
    patterns_len: usize,
) -> i64 {
    let caller_pid = crate::scheduler::with_scheduler(|s| s.current_pid());

    let perm_decision = match decision {
        0 => crate::permission::ipc_channel::PermDecision::Denied,
        1 => {
            // Parse granted patterns from userspace buffer.
            if patterns_len > 0 {
                if !validate_user_pointer(patterns_ptr as u64, patterns_len) {
                    return EFAULT;
                }
                let data = core::slice::from_raw_parts(patterns_ptr, patterns_len);
                let text = match core::str::from_utf8(data) {
                    Ok(s) => s,
                    Err(_) => return EINVAL,
                };
                let patterns: alloc::vec::Vec<crate::permission::PathPattern> = text
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::permission::PathPattern::new(alloc::string::String::from(s)))
                    .collect();
                crate::permission::ipc_channel::PermDecision::Granted(patterns)
            } else {
                crate::permission::ipc_channel::PermDecision::Granted(alloc::vec::Vec::new())
            }
        }
        2 => crate::permission::ipc_channel::PermDecision::GrantedInherited,
        _ => return EINVAL,
    };

    let blocked_pid = crate::process::Pid::new(blocked_pid_index as u16, 1);

    match crate::permission::ipc_channel::submit_response(caller_pid, blocked_pid, perm_decision) {
        Ok(()) => {
            // Unblock the waiting process.
            crate::scheduler::with_scheduler(|scheduler| {
                scheduler.unblock(blocked_pid);
            });
            0
        }
        Err(e) => e,
    }
}
