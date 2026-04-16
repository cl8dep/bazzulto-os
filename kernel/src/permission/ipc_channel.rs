// permission/ipc_channel.rs — Dedicated kernel ↔ permissiond IPC channel.
//
// This module provides a secure, one-way communication channel from the kernel
// to the permissiond daemon, plus a response syscall for permissiond to reply.
//
// Architecture:
//   1. permissiond calls sys_register_permissiond() at boot.
//      → The kernel records its PID and creates a ring buffer.
//      → permissiond receives an fd for reading permission requests.
//
//   2. When exec() needs a permission decision:
//      → The kernel writes a PermRequest to the ring buffer.
//      → The kernel blocks the exec'ing process.
//      → permissiond reads the request from its fd.
//
//   3. permissiond evaluates the request (policy lookup, user prompt).
//      → permissiond calls sys_perm_respond(pid, decision).
//      → The kernel unblocks the exec'ing process with the decision.
//
// Security:
//   - Only the kernel writes to the ring buffer (no userspace write path).
//   - Only one process can register as permissiond (first-come, enforced).
//   - sys_perm_respond only accepts calls from the registered permissiond PID.
//   - The ring buffer fd is a special kernel-internal descriptor; it cannot be
//     dup'd, forked, or passed via SCM_RIGHTS.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::process::Pid;

/// Maximum pending permission requests.
const MAX_PENDING_REQUESTS: usize = 16;

/// A permission request from the kernel to permissiond.
#[derive(Clone)]
pub struct PermRequest {
    /// PID of the blocked process (waiting for a decision).
    pub blocked_pid: Pid,
    /// Binary path being exec'd.
    pub binary_path: String,
    /// SHA-256 hex hash of the binary's PT_LOAD segments.
    pub hash_hex: String,
    /// Whether the binary has a .bazzulto_permissions ELF section.
    pub has_elf_section: bool,
    /// UID of the calling process.
    pub caller_uid: u32,
    /// Whether the calling process has a TTY (for interactive prompt).
    pub has_tty: bool,
}

/// The decision from permissiond back to the kernel.
#[derive(Clone, Debug)]
pub enum PermDecision {
    /// Grant the declared/inherited permissions.  Process may proceed.
    Granted(Vec<super::PathPattern>),
    /// Grant inherited permissions (Tier 4 user approval).
    GrantedInherited,
    /// Deny execution.  Return EPERM to the caller.
    Denied,
}

/// Global singleton for the permissiond IPC channel.
struct PermissiondChannel {
    /// PID of the registered permissiond process, or None if not yet registered.
    registered_pid: Option<Pid>,
    /// Ring buffer of pending requests.
    pending: [Option<PermRequest>; MAX_PENDING_REQUESTS],
    /// Number of pending requests.
    pending_count: usize,
    /// Responses from permissiond, keyed by blocked PID index.
    responses: [Option<PermDecision>; MAX_PENDING_REQUESTS],
}

impl PermissiondChannel {
    const fn new() -> Self {
        const NONE_REQ: Option<PermRequest> = None;
        const NONE_DEC: Option<PermDecision> = None;
        PermissiondChannel {
            registered_pid: None,
            pending: [NONE_REQ; MAX_PENDING_REQUESTS],
            pending_count: 0,
            responses: [NONE_DEC; MAX_PENDING_REQUESTS],
        }
    }
}

struct ChannelWrapper(UnsafeCell<PermissiondChannel>);
unsafe impl Sync for ChannelWrapper {}

static CHANNEL: ChannelWrapper = ChannelWrapper(UnsafeCell::new(PermissiondChannel::new()));

fn channel() -> &'static mut PermissiondChannel {
    unsafe { &mut *CHANNEL.0.get() }
}

// ---------------------------------------------------------------------------
// Public API — called from syscall handlers
// ---------------------------------------------------------------------------

/// Register the calling process as permissiond.
///
/// Returns `Ok(())` on success, `Err(errno)` if already registered or
/// the caller lacks the RegisterPermissiond action permission.
pub fn register_permissiond(pid: Pid) -> Result<(), i64> {
    let ch = channel();
    if ch.registered_pid.is_some() {
        return Err(-1); // EPERM — already registered
    }
    ch.registered_pid = Some(pid);
    crate::drivers::uart::puts("[bpm] permissiond registered as PID ");
    crate::drivers::uart::put_hex(pid.index as u64);
    crate::drivers::uart::puts("\r\n");
    Ok(())
}

/// Check if permissiond is registered and available.
pub fn is_permissiond_available() -> bool {
    channel().registered_pid.is_some()
}

/// Submit a permission request from the kernel.
///
/// Returns the slot index for later response matching, or `None` if the
/// queue is full.  The calling process should be blocked after this.
pub fn submit_request(request: PermRequest) -> Option<usize> {
    let ch = channel();
    if ch.pending_count >= MAX_PENDING_REQUESTS {
        return None; // Queue full — fall back to Tier 4 inheritance.
    }
    // Find an empty slot.
    for (i, slot) in ch.pending.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(request);
            ch.pending_count += 1;
            return Some(i);
        }
    }
    None
}

/// Read the next pending request (called by permissiond via syscall).
///
/// Returns `None` if no requests are pending.
pub fn read_next_request(caller_pid: Pid) -> Option<PermRequest> {
    let ch = channel();
    // Only the registered permissiond may read.
    if ch.registered_pid != Some(caller_pid) {
        return None;
    }
    for slot in ch.pending.iter_mut() {
        if let Some(req) = slot.take() {
            ch.pending_count -= 1;
            return Some(req);
        }
    }
    None
}

/// Submit a response from permissiond for a blocked process.
///
/// Only the registered permissiond PID may call this.
/// Returns `Ok(())` if the response was accepted.
pub fn submit_response(
    caller_pid: Pid,
    blocked_pid: Pid,
    decision: PermDecision,
) -> Result<(), i64> {
    let ch = channel();
    if ch.registered_pid != Some(caller_pid) {
        return Err(-1); // EPERM — not permissiond
    }
    // Store the response keyed by blocked PID index.
    let slot = (blocked_pid.index as usize) % MAX_PENDING_REQUESTS;
    ch.responses[slot] = Some(decision);
    Ok(())
}

/// Check if a response is available for a blocked process.
///
/// Returns `Some(decision)` and removes it from the store.
pub fn take_response(blocked_pid: Pid) -> Option<PermDecision> {
    let ch = channel();
    let slot = (blocked_pid.index as usize) % MAX_PENDING_REQUESTS;
    ch.responses[slot].take()
}

/// Return the registered permissiond PID, if any.
pub fn permissiond_pid() -> Option<Pid> {
    channel().registered_pid
}

/// Clear the permissiond registration.
///
/// Called from sys_exit when the permissiond process dies.
/// Allows a new permissiond to register on restart.
pub fn unregister_permissiond(pid: Pid) {
    let ch = channel();
    if ch.registered_pid == Some(pid) {
        ch.registered_pid = None;
        crate::drivers::uart::puts("[bpm] permissiond unregistered (process exited)\r\n");
    }
}
