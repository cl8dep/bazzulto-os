// sync/mod.rs — Kernel synchronisation primitives.
//
// Currently provides a ticket spinlock suitable for SMP use.
// IRQ-save wrappers will be added here when SMP bring-up requires them.

pub mod spinlock;

pub use spinlock::SpinLock;
pub use spinlock::SpinLockGuard;
