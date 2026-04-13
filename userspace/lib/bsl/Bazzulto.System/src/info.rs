//! OS and hardware information.

use crate::raw;
use crate::time::{Duration, SystemTime};

pub struct Info;

impl Info {
    pub fn os_name() -> &'static str {
        "Bazzulto"
    }

    pub fn os_version() -> &'static str {
        "1.0.0"
    }

    pub fn kernel_version() -> &'static str {
        "0.1.0-dev"
    }

    pub fn arch() -> &'static str {
        "aarch64"
    }

    pub fn pid() -> i32 {
        raw::raw_getpid() as i32
    }

    pub fn ppid() -> i32 {
        raw::raw_getppid() as i32
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn cpu_brand() -> Option<&'static str> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn cpu_count() -> Option<u32> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn memory_total() -> Option<u64> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn uptime() -> Option<Duration> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn boot_time() -> Option<SystemTime> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn executable() -> Option<alloc::string::String> {
        None
    }
}
