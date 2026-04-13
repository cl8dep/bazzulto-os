//! Slab allocator for Bazzulto userspace.
//!
//! # Architecture
//!
//! Small allocations (≤ 4096 bytes) are carved from 64 KB arenas obtained via
//! `mmap`.  Each power-of-2 size class (8–4096) has an independent arena and
//! a per-class free list.  Freed cells are returned to the free list; a new
//! arena is only requested from the kernel when the current one is exhausted.
//!
//! Large allocations (> 4096 bytes) use a direct `mmap`/`munmap` pair, with
//! the total mapped size stored in an 8-byte header immediately before the
//! returned pointer so `dealloc` can call `munmap` with the correct length.
//!
//! # Comparison with the previous allocator
//!
//! The previous allocator issued one `mmap` per `alloc` call.  A 64 KB arena
//! holds 8 192 cells of 8 bytes, so for small allocations this allocator
//! reduces `mmap` syscalls by up to three orders of magnitude.  The kernel's
//! region tracking table (currently a flat Vec) is therefore not stressed by
//! normal workloads.
//!
//! # Thread safety
//!
//! A test-and-set spinlock (`AtomicBool`) protects all state.  Suitable for
//! single-threaded processes and for multi-threaded processes with low
//! contention.  The spinlock is released before any blocking operation
//! (there are none — mmap is non-blocking on Bazzulto).
//!
//! # Size classes
//!
//! Index │ Cell size
//! ──────┼──────────
//!     0 │      8 B
//!     1 │     16 B
//!     2 │     32 B
//!     3 │     64 B
//!     4 │    128 B
//!     5 │    256 B
//!     6 │    512 B
//!     7 │   1024 B
//!     8 │   2048 B
//!     9 │   4096 B
//! > 4096 │ large path (direct mmap)

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};
use crate::raw;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of 4 KB pages per arena — 64 KB total.
const ARENA_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;

/// Total bytes in one arena.
const ARENA_SIZE: usize = ARENA_PAGES * PAGE_SIZE; // 65 536

/// Size classes are powers of 2 starting at 8 bytes.
const NUM_SIZE_CLASSES: usize = 10; // 8 … 4096

/// Allocations larger than this threshold bypass the slab and use mmap/munmap
/// directly.
const LARGE_THRESHOLD: usize = PAGE_SIZE; // 4096

/// Bytes reserved at the start of a large-allocation mmap region to store the
/// region length (so `dealloc` can call `munmap` with the correct size).
const LARGE_HEADER_SIZE: usize = 8;

// mmap prot / flags constants (match kernel syscall/mod.rs).
const PROT_READ_WRITE: i32 = 0x1 | 0x2;
const MAP_ANONYMOUS_PRIVATE: i32 = 0x20 | 0x02;

// ---------------------------------------------------------------------------
// Size-class arithmetic
// ---------------------------------------------------------------------------

/// Round `n` up to the smallest power of two that is ≥ 8 and ≥ `n`.
#[inline]
const fn round_up_to_class_size(n: usize) -> usize {
    if n <= 8 {
        return 8;
    }
    // Bit-trick: next power of 2 ≥ n.
    let mut v = n - 1;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v |= v >> 32;
    v + 1
}

/// Size-class index for `cell_size`.  `cell_size` must be a power of two in
/// [8, 4096]; the result is in [0, 9].
#[inline]
const fn class_index(cell_size: usize) -> usize {
    cell_size.trailing_zeros() as usize - 3
}

/// Cell size for class `index` (inverse of `class_index`).
#[inline]
const fn cell_size_for_index(index: usize) -> usize {
    8 << index
}

// ---------------------------------------------------------------------------
// Per-class arena state
// ---------------------------------------------------------------------------

/// Runtime state for one size class.
///
/// Each class maintains:
/// - A bump pointer into the current arena.
/// - A free list of previously freed cells (embedded linked list — each freed
///   cell stores the address of the next free cell in its first 8 bytes).
///
/// When the bump pointer reaches the end of the arena, a new 64 KB arena is
/// obtained from the kernel.  Old arenas are never returned (acceptable trade-
/// off for a first implementation; the kernel's physical allocator reuses the
/// pages when the process exits).
struct SizeClass {
    /// Bump pointer — next allocation comes from here.
    bump: usize,
    /// One-past-end of the current arena.
    end: usize,
    /// Head of the free list.  0 = empty.
    free_head: usize,
}

impl SizeClass {
    const fn new() -> Self {
        SizeClass { bump: 0, end: 0, free_head: 0 }
    }

    /// Allocate one cell.  Returns null on kernel `mmap` failure.
    ///
    /// # Safety
    /// Caller must hold the allocator spinlock.
    unsafe fn alloc(&mut self, cell_size: usize) -> *mut u8 {
        // Fast path: pop from free list.
        if self.free_head != 0 {
            let cell = self.free_head as *mut usize;
            // Read next pointer stored in the cell's first word.
            self.free_head = unsafe { *cell };
            return cell as *mut u8;
        }

        // Bump path: serve from current arena.
        if self.bump + cell_size <= self.end {
            let ptr = self.bump as *mut u8;
            self.bump += cell_size;
            return ptr;
        }

        // Arena exhausted — request a fresh one from the kernel.
        let result = raw::raw_mmap(0, ARENA_SIZE as u64, PROT_READ_WRITE, MAP_ANONYMOUS_PRIVATE);
        if result < 0 {
            return core::ptr::null_mut();
        }
        let base = result as usize;
        self.bump = base + cell_size;
        self.end = base + ARENA_SIZE;
        base as *mut u8
    }

    /// Return `ptr` to the free list.
    ///
    /// # Safety
    /// `ptr` must have been returned by `alloc` on this class and not yet freed.
    /// Caller must hold the allocator spinlock.
    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        let cell = ptr as *mut usize;
        // Write the current free-list head into the cell's first word.
        unsafe { *cell = self.free_head };
        self.free_head = ptr as usize;
    }
}

// ---------------------------------------------------------------------------
// Allocator state
// ---------------------------------------------------------------------------

struct AllocatorState {
    classes: [SizeClass; NUM_SIZE_CLASSES],
}

impl AllocatorState {
    const fn new() -> Self {
        AllocatorState {
            // Const array initialiser — SizeClass is not Copy so must be
            // written out individually.
            classes: [
                SizeClass::new(), SizeClass::new(), SizeClass::new(),
                SizeClass::new(), SizeClass::new(), SizeClass::new(),
                SizeClass::new(), SizeClass::new(), SizeClass::new(),
                SizeClass::new(),
            ],
        }
    }

    /// Allocate for a small layout (effective size ≤ `LARGE_THRESHOLD`).
    ///
    /// # Safety
    /// Caller must hold the allocator spinlock.
    unsafe fn alloc_small(&mut self, effective_size: usize) -> *mut u8 {
        let cell_size = round_up_to_class_size(effective_size);
        let index = class_index(cell_size);
        unsafe { self.classes[index].alloc(cell_size) }
    }

    /// Return a small allocation to its size class.
    ///
    /// # Safety
    /// `ptr` must be a valid small allocation with the same layout.
    /// Caller must hold the allocator spinlock.
    unsafe fn dealloc_small(&mut self, ptr: *mut u8, effective_size: usize) {
        let cell_size = round_up_to_class_size(effective_size);
        let index = class_index(cell_size);
        unsafe { self.classes[index].dealloc(ptr) };
    }
}

// ---------------------------------------------------------------------------
// Large allocation helpers (no slab — direct mmap / munmap)
// ---------------------------------------------------------------------------

/// Allocate `size` bytes for a large allocation (bypasses slab).
///
/// Layout of the mmap region:
/// ```
/// [ 8-byte total_length ][ size bytes of user data ][ padding to page boundary ]
/// ```
///
/// # Safety
/// No lock required — does not touch `AllocatorState`.
unsafe fn alloc_large(size: usize) -> *mut u8 {
    let total = round_up_to_page(size + LARGE_HEADER_SIZE);
    let result = raw::raw_mmap(0, total as u64, PROT_READ_WRITE, MAP_ANONYMOUS_PRIVATE);
    if result < 0 {
        return core::ptr::null_mut();
    }
    let base = result as usize;
    // Store the total mmap length so dealloc can call munmap correctly.
    unsafe { *(base as *mut usize) = total };
    (base + LARGE_HEADER_SIZE) as *mut u8
}

/// Free a large allocation returned by `alloc_large`.
///
/// # Safety
/// `ptr` must be a valid large allocation.
unsafe fn dealloc_large(ptr: *mut u8) {
    let base = (ptr as usize) - LARGE_HEADER_SIZE;
    let total = unsafe { *(base as *mut usize) };
    raw::raw_munmap(base as u64, total as u64);
}

#[inline]
fn round_up_to_page(size: usize) -> usize {
    (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

// ---------------------------------------------------------------------------
// Spinlock
// ---------------------------------------------------------------------------

static LOCK: AtomicBool = AtomicBool::new(false);

#[inline]
fn lock_acquire() {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[inline]
fn lock_release() {
    LOCK.store(false, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

// Safety: all access is serialised through LOCK (acquire/release ordering).
static mut STATE: AllocatorState = AllocatorState::new();

/// Return a raw pointer to the allocator state.
///
/// Using `addr_of_mut!` avoids creating a mutable reference to a mutable
/// static (which triggers the `static_mut_refs` lint in Rust 2024).  The
/// resulting raw pointer is only dereferenced while the spinlock is held.
#[inline]
fn state_ptr() -> *mut AllocatorState {
    core::ptr::addr_of_mut!(STATE)
}

// ---------------------------------------------------------------------------
// GlobalAlloc implementation
// ---------------------------------------------------------------------------

/// Bazzulto global slab allocator.
pub struct BazzultoAllocator;

unsafe impl GlobalAlloc for BazzultoAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Effective size must satisfy alignment.  For the common case (align ≤
        // size), effective == size.  For exotic alignments (align > size), we
        // round up so the bump pointer naturally satisfies alignment because
        // arenas are page-aligned and size classes are powers of two.
        let effective = layout.size().max(layout.align());

        if effective > LARGE_THRESHOLD {
            // Large allocations bypass the slab — no lock needed.
            return unsafe { alloc_large(effective) };
        }

        lock_acquire();
        let ptr = unsafe { (*state_ptr()).alloc_small(effective) };
        lock_release();
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let effective = layout.size().max(layout.align());

        if effective > LARGE_THRESHOLD {
            unsafe { dealloc_large(ptr) };
            return;
        }

        lock_acquire();
        unsafe { (*state_ptr()).dealloc_small(ptr, effective) };
        lock_release();
    }
}

#[global_allocator]
static ALLOCATOR: BazzultoAllocator = BazzultoAllocator;

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    raw::raw_exit(1)
}
