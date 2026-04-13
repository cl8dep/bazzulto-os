// sync/spinlock.rs — Ticket spinlock for AArch64 SMP.
//
// Uses atomic fetch-add (which compiles to LDADD or CAS loop with acquire/release
// semantics on AArch64) for correctness on the weakly-ordered ARM memory model.
//
// A ticket lock guarantees FIFO ordering: each `lock()` caller receives a
// monotonically increasing ticket and waits until `now_serving` equals its
// ticket.  This prevents starvation on contended locks.
//
// Reference: ARM ARM DDI 0487 §B2.10 (exclusive monitors),
//            §B2.9 (acquire/release semantics).

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// SpinLock<T>
// ---------------------------------------------------------------------------

/// A ticket spinlock protecting a value of type `T`.
///
/// On AArch64 `acquire` and `release` atomic orderings map directly to
/// load-acquire (LDAR) and store-release (STLR) instructions, providing
/// the necessary memory barrier without a separate DMB instruction.
pub struct SpinLock<T> {
    /// Next ticket to be issued.  Incremented (with Acquire) by each `lock()`.
    next_ticket: AtomicU32,
    /// Ticket currently being served.  Incremented (with Release) on `drop`.
    now_serving: AtomicU32,
    data: UnsafeCell<T>,
}

// SAFETY: SpinLock provides mutual exclusion, so sharing across threads/cores
// is safe as long as T itself can be sent between threads.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Construct a new spinlock wrapping `data`.
    ///
    /// `const fn` so it can be used to initialise `static` variables.
    pub const fn new(data: T) -> Self {
        Self {
            next_ticket: AtomicU32::new(0),
            now_serving: AtomicU32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, spinning until our ticket is served.
    ///
    /// Returns a `SpinLockGuard` that releases the lock when dropped.
    ///
    /// `fetch_add(Acquire)` issues a load-acquire (LDADD with .A modifier on
    /// ARMv8.1+ LSE, or LDAXR loop on ARMv8.0), ensuring that all memory
    /// accesses in the critical section are ordered after the lock acquisition.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // Take a ticket.  The Acquire ordering ensures we see all stores made
        // by the previous lock holder before we enter the critical section.
        let ticket = self.next_ticket.fetch_add(1, Ordering::Acquire);

        // Spin until the lock is handed to our ticket.
        // `Acquire` on the load pairs with the `Release` store in drop().
        while self.now_serving.load(Ordering::Acquire) != ticket {
            // Hint to the CPU that this is a spin-wait loop.
            // On AArch64 this emits `YIELD`, allowing the hardware to
            // de-prioritise the stalled pipeline and reduce contention on the
            // cache line holding `now_serving`.
            core::hint::spin_loop();
        }

        SpinLockGuard { lock: self, ticket }
    }

    /// Release the lock without going through the normal `Drop` path.
    ///
    /// # Safety
    /// Must only be called when the lock is currently held by this CPU and
    /// the guard has been forgotten (e.g., via `core::mem::forget`).  Typical
    /// use: panic handlers and early-boot IRQ paths where RAII cannot run.
    pub unsafe fn force_unlock(&self) {
        self.now_serving.fetch_add(1, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// SpinLockGuard<'a, T>
// ---------------------------------------------------------------------------

/// RAII guard returned by `SpinLock::lock()`.
///
/// Releases the lock when dropped by incrementing `now_serving`.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    /// The ticket we were issued.  Stored for debugging; not used in release.
    #[allow(dead_code)]
    ticket: u32,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: we hold the lock, so exclusive access is guaranteed.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: we hold the lock, so exclusive mutable access is guaranteed.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        // Increment `now_serving` with Release ordering so that all stores
        // made inside the critical section are visible to the next lock holder
        // before it observes the updated `now_serving`.
        //
        // On AArch64 this compiles to STADDL (LSE) or a STLR-based loop,
        // which is a store-release — no additional DMB is needed.
        self.lock.now_serving.fetch_add(1, Ordering::Release);
    }
}
