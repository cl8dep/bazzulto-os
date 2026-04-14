//! OS and hardware information — sourced from the kernel, not hardcoded.
//!
//! `Info::os_name()`, `os_version()`, `kernel_version()`, and `arch()` call
//! `sys_uname` (slot 41) and cache the result in process-local statics so
//! subsequent calls are free (no syscall).
//!
//! `Info::cpu_count()`, `memory_total()`, `uptime()` call `sys_sysinfo`
//! (slot 42).  These are not cached — each call makes one syscall.
//!
//! Kernel buffer layouts:
//!   uname (390 bytes): 6 fields × 65 bytes, NUL-terminated
//!     [0..65]    sysname   (→ os_name)
//!     [65..130]  nodename
//!     [130..195] release   (→ kernel_version)
//!     [195..260] version   (→ os_version / build description)
//!     [260..325] machine   (→ arch)
//!     [325..390] domainname
//!
//!   sysinfo (32 bytes): 4 × u64
//!     [0]  uptime_seconds
//!     [1]  total_ram_bytes
//!     [2]  free_ram_bytes
//!     [3]  process_count

extern crate alloc;

use crate::raw;
use crate::time::{Duration, SystemTime};
use core::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// uname cache — populated once on first call, then read from statics
// ---------------------------------------------------------------------------

const FIELD_LEN: usize = 65;
const UNAME_BUF_LEN: usize = FIELD_LEN * 6; // 390 bytes

// Each field is stored as a fixed-size byte array.  The stored bytes are
// NUL-terminated (kernel contract), so we can return &str slices into them.
static UNAME_LOADED: AtomicBool = AtomicBool::new(false);

// SAFETY: written once (under the UNAME_LOADED flag) before any read.
static mut UNAME_BUF: [u8; UNAME_BUF_LEN] = [0u8; UNAME_BUF_LEN];

/// Ensure the uname buffer has been fetched from the kernel.
///
/// Uses a simple flag — safe because Bazzulto userspace processes are
/// single-threaded.  The AtomicBool provides the memory-ordering guarantee
/// that the buffer write is visible before the flag is set.
fn ensure_uname_loaded() {
    if UNAME_LOADED.load(Ordering::Acquire) {
        return;
    }
    let result = unsafe { raw::raw_uname(UNAME_BUF.as_mut_ptr()) };
    if result == 0 {
        UNAME_LOADED.store(true, Ordering::Release);
    }
    // If the syscall fails the buffer stays zeroed — callers get empty strings,
    // which is safe and avoids a panic in early-boot contexts.
}

/// Return a `&'static str` slice of field `index` (0–5) from the uname cache.
///
/// Strips the NUL terminator.
fn uname_field(index: usize) -> &'static str {
    ensure_uname_loaded();
    let start = index * FIELD_LEN;
    let field = unsafe { &UNAME_BUF[start..start + FIELD_LEN] };
    // Find the NUL terminator.
    let nul = field.iter().position(|&b| b == 0).unwrap_or(FIELD_LEN);
    // SAFETY: the kernel writes valid UTF-8 into uname fields (ASCII names).
    unsafe { core::str::from_utf8_unchecked(&field[..nul]) }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// OS and hardware information, sourced from the kernel at runtime.
pub struct Info;

impl Info {
    /// Operating system name (e.g. `"Bazzulto"`).
    ///
    /// Sourced from `sys_uname` field `sysname`.
    pub fn os_name() -> &'static str {
        uname_field(0)
    }

    /// OS release/version string (e.g. `"Bazzulto 0.1.0 (AArch64)"`).
    ///
    /// Sourced from `sys_uname` field `version` (index 3), which contains
    /// the human-readable build description.
    pub fn os_version() -> &'static str {
        uname_field(3)
    }

    /// Kernel release version (e.g. `"0.1.0"`).
    ///
    /// Sourced from `sys_uname` field `release` (index 2).
    pub fn kernel_version() -> &'static str {
        uname_field(2)
    }

    /// CPU architecture (e.g. `"aarch64"`).
    ///
    /// Sourced from `sys_uname` field `machine` (index 4).
    pub fn arch() -> &'static str {
        uname_field(4)
    }

    /// PID of the current process.
    pub fn pid() -> i32 {
        raw::raw_getpid() as i32
    }

    /// PPID of the current process.
    pub fn ppid() -> i32 {
        raw::raw_getppid() as i32
    }

    /// CPU brand string — not available from kernel yet.
    pub fn cpu_brand() -> Option<&'static str> {
        None
    }

    /// Number of online CPUs.
    ///
    /// Not yet reported by `sys_sysinfo`; returns `None` until the kernel
    /// extends the sysinfo buffer with a cpu_count field.
    pub fn cpu_count() -> Option<u32> {
        None
    }

    /// Total physical RAM in bytes.
    ///
    /// Sourced from `sys_sysinfo` field `total_ram_bytes` (index 1).
    pub fn memory_total() -> Option<u64> {
        let mut buf = [0u64; 4];
        let result = raw::raw_sysinfo(buf.as_mut_ptr());
        if result == 0 && buf[1] > 0 { Some(buf[1]) } else { None }
    }

    /// System uptime as a `Duration`.
    ///
    /// Sourced from `sys_sysinfo` field `uptime_seconds` (index 0).
    pub fn uptime() -> Option<Duration> {
        let mut buf = [0u64; 4];
        let result = raw::raw_sysinfo(buf.as_mut_ptr());
        if result == 0 { Some(Duration::secs(buf[0])) } else { None }
    }

    /// Boot time — derived from `SystemTime::now()` minus uptime.
    pub fn boot_time() -> Option<SystemTime> {
        let uptime_secs = Self::uptime()?.as_secs();
        let now = SystemTime::now();
        if now.secs >= uptime_secs {
            Some(SystemTime { secs: now.secs - uptime_secs, nanos: 0 })
        } else {
            None
        }
    }

    /// Path to the current executable — not yet implemented.
    pub fn executable() -> Option<alloc::string::String> {
        None
    }
}
