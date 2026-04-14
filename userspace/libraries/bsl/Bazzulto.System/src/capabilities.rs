//! Process capability constants.
//!
//! A capability is a bit in the process's capability mask. A process may only
//! exercise privileged operations for which it holds the corresponding bit.
//!
//! # Granting capabilities
//!
//! Use `raw_spawn_with_capabilities` to spawn a child with a specific mask.
//! The calling process must hold `CAP_SETCAP` and every capability it grants.
//!
//! # Kernel enforcement
//!
//! The kernel verifies the relevant capability bit in each guarded syscall:
//!   - `sys_framebuffer_map` (syscall 70) — requires `CAP_DISPLAY`

/// Map the boot-time framebuffer into the process's address space.
///
/// Should only be granted to the display server (`bzdisplayd`).
pub const CAP_DISPLAY: u64 = 1 << 0;

/// Grant capabilities to child processes via `sys_spawn`.
///
/// Only `bzinit` holds this at boot. A process with `CAP_SETCAP` may pass
/// any subset of its own capabilities to processes it spawns.
pub const CAP_SETCAP: u64 = 1 << 1;

/// Convenience: no capabilities.
pub const CAP_NONE: u64 = 0;

/// Convenience: all currently defined capabilities.
pub const CAP_ALL: u64 = CAP_DISPLAY | CAP_SETCAP;
