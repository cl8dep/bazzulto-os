//! Thread API — mostly deferred in v1.0 (single-threaded kernel).

use crate::time::Duration;
use crate::raw;

// ---------------------------------------------------------------------------
// ThreadId
// ---------------------------------------------------------------------------

pub struct ThreadId(i32);

// ---------------------------------------------------------------------------
// Thread facade
// ---------------------------------------------------------------------------

pub struct Thread;

impl Thread {
    /// Handle to the current thread.
    pub fn current() -> CurrentThread {
        CurrentThread
    }

    /// Sleep for the given duration.
    pub fn sleep(duration: Duration) -> Result<(), i32> {
        crate::time::Time::sleep(duration)
    }

    /// Yield the current time slice.
    pub fn yield_now() {
        raw::raw_yield();
    }

    /// Deferred — requires kernel thread support (see docs/tech-debt/bzinit-v1.md).
    pub fn spawn<F: FnOnce()>(_f: F) -> Result<ThreadHandle, i32> {
        Err(-38) // ENOSYS
    }
}

// ---------------------------------------------------------------------------
// CurrentThread
// ---------------------------------------------------------------------------

pub struct CurrentThread;

impl CurrentThread {
    pub fn id(&self) -> ThreadId {
        ThreadId(raw::raw_getpid() as i32)
    }

    /// Always "main" in v1.0.
    pub fn name(&self) -> &'static str {
        "main"
    }

    /// Deferred — requires kernel thread naming support (see docs/tech-debt/bzinit-v1.md).
    pub fn set_name(&self, _name: &str) {}
}

// ---------------------------------------------------------------------------
// ThreadHandle
// ---------------------------------------------------------------------------

pub struct ThreadHandle;

impl ThreadHandle {
    /// Deferred — requires kernel thread support (see docs/tech-debt/bzinit-v1.md).
    pub fn join(self) -> Result<(), i32> {
        Err(-38) // ENOSYS
    }
}

// ---------------------------------------------------------------------------
// ThreadPool
// ---------------------------------------------------------------------------

/// Thread pool — deferred. All methods return Err(-38).
pub struct ThreadPool {
    workers: usize,
}

impl ThreadPool {
    pub fn new(workers: usize) -> ThreadPool {
        ThreadPool { workers }
    }

    /// Deferred — requires kernel thread support (see docs/tech-debt/bzinit-v1.md).
    pub fn execute<F: FnOnce() + 'static>(&self, _f: F) -> Result<(), i32> {
        Err(-38) // ENOSYS
    }
}
