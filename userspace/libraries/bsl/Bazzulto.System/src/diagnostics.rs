//! Diagnostics — Stopwatch, Diagnostics facade, DiagProcess.

use crate::time::{Time, Duration, Instant};
use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Stopwatch
// ---------------------------------------------------------------------------

/// A wall-clock stopwatch.
pub struct Stopwatch {
    start: Instant,
}

impl Stopwatch {
    pub fn start() -> Stopwatch {
        Stopwatch { start: Time::now() }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis()
    }

    pub fn reset(&mut self) {
        self.start = Time::now();
    }
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

pub struct Diagnostics;

impl Diagnostics {
    /// Deferred — requires debugger detection support (see docs/tech-debt/bzinit-v1.md).
    pub fn is_debugger_attached() -> bool {
        false
    }

    /// No-op — deferred.
    pub fn break_if_debugging() {}

    /// Panics with `msg` if `condition` is false.
    pub fn assert(condition: bool, msg: &str) {
        if !condition {
            panic!("{}", msg);
        }
    }

    /// No-op — deferred (see docs/tech-debt/bzinit-v1.md).
    pub fn print_stack_trace() {}
}

// ---------------------------------------------------------------------------
// DiagProcess
// ---------------------------------------------------------------------------

/// Per-process diagnostic information.
pub struct DiagProcess {
    pub pid: i32,
}

impl DiagProcess {
    pub fn current() -> DiagProcess {
        DiagProcess { pid: crate::raw::raw_getpid() as i32 }
    }

    pub fn find_by_pid(pid: i32) -> Option<DiagProcess> {
        Some(DiagProcess { pid })
    }

    /// Deferred — requires kernel process search (see docs/tech-debt/bzinit-v1.md).
    pub fn find_by_name(_name: &str) -> Option<DiagProcess> {
        None
    }

    /// Deferred — requires kernel process list (see docs/tech-debt/bzinit-v1.md).
    pub fn list() -> Vec<DiagProcess> {
        Vec::new()
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn cpu_usage(&self) -> Option<u32> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn memory_usage(&self) -> Option<u64> {
        None
    }

    /// Deferred — requires kernel thread list syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn threads(&self) -> Vec<i32> {
        Vec::new()
    }

    /// Deferred — requires kernel fd list syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn open_files(&self) -> Vec<String> {
        Vec::new()
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn start_time(&self) -> Option<crate::time::SystemTime> {
        None
    }
}
