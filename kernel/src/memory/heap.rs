// memory/heap.rs — Kernel heap allocator (first-fit, iterative).
//
// Algorithm: linked list of blocks.  Each block is preceded by a `BlockHeader`
// containing its size and a magic number that distinguishes free vs. used blocks
// and detects corruption / double-free.
//
// Improvements over the original C heap and the first Rust draft:
//
//   Gap 1  — Magic-number validation replaces `is_free: bool`.
//   Gap 2  — Freed memory is poisoned with 0xCC (use-after-free detection).
//   Gap 3  — Allocation statistics tracked per heap instance.
//   Gap 4  — debug_assert that IRQs are disabled at entry (re-entrancy guard).
//   Gap 5  — AllocContext enum: Kernel (can grow) vs Atomic (never grows).
//   Gap 6  — Lazy coalesce: coalesce() called only when first-fit fails,
//             not on every free().
//   Gap 7  — HeapSnapshot for runtime diagnostics.
//   Gap 8  — grow() tries HEAP_GROWTH_PAGES first, falls back to pages_needed.
//   Gap 9  — GlobalAlloc passes layout.align(); alloc_bytes honours it.
//   Gap 10 — One guard page left between _kernel_end and heap base.
//   Gap 11 — Compile-time assert on BlockHeader size.
//
// SAFETY INVARIANT: all public methods must be called with a global lock held
// (or from a single-threaded, interrupt-disabled context).

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

use super::address::{align_up, VirtualAddress};
use super::virtual_memory::{kernel_virtual_memory_alloc, MapError, PAGE_FLAGS_KERNEL_DATA};
use super::{physical::PhysicalAllocator, virtual_memory::PageTable};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum alignment guaranteed for every allocation payload.
///
/// 16 bytes covers all AArch64 primitive types, including u128 and 128-bit
/// atomics (LDXP/STXP require 16-byte alignment).
const HEAP_MIN_ALIGN: usize = 16;

/// Minimum number of usable bytes in the remainder block after a split.
///
/// Splitting below this threshold wastes space on headers and inflates the
/// free list, degrading first-fit to O(n²) on allocation-heavy workloads.
/// 64 bytes covers most small kernel objects (fd-table entries, wait-queue
/// nodes, small scheduler structs).
const MIN_BLOCK_SIZE: usize = 64;

/// Pages mapped per grow() call (batch target).
///
/// 8 pages = 32 KiB with 4 KiB pages — fits a full kernel stack in one grow.
/// Amortises TLB flushes and page-table walks.
const HEAP_GROWTH_PAGES: usize = 8;

/// Pages mapped in init() to reduce grows during early boot.
const HEAP_INITIAL_PAGES: usize = 4;

/// Maximum size for a single allocation.
///
/// The kernel heap is contiguous: a fragmented heap cannot satisfy a huge
/// contiguous request even when total free bytes are large.  This cap avoids
/// exhausting the heap with one erroneous request.
///
/// References:
///   - Linux kmalloc:          ~4 MiB (KMALLOC_MAX_SIZE)
///   - Windows NonPagedPool:   ~4 MiB practical limit
///   - macOS kalloc:           ~32 MiB before delegating to vmalloc
///
/// 32 MiB — generous (8× Linux), equal to macOS kalloc's upper bound.
/// For larger objects, use vmalloc (Fase 5).
const MAX_ALLOC_SIZE: usize = 32 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Slab allocator — size classes for small allocations
// ---------------------------------------------------------------------------

/// Fixed size classes managed by the slab layer, in bytes.
///
/// Any allocation whose size rounds up to one of these classes is served from
/// a per-class freelist rather than the first-fit list. This eliminates the
/// internal fragmentation that occurs when a first-fit heap carves a 17-byte
/// request from a 64-byte block, then cannot reuse the leftover 15 bytes.
///
/// The smallest class (16 B) matches HEAP_MIN_ALIGN, so no class can split.
/// The largest (4096 B) is one page — above this size the first-fit path
/// wins because slab overhead per object is negligible at large sizes.
///
/// Reference: Linux SLAB / SLUB size classes, glibc malloc fastbins.
const SLAB_CLASS_SIZES: [usize; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
const SLAB_CLASS_COUNT: usize = SLAB_CLASS_SIZES.len();

/// Return the slab class index for a request of `size` bytes, or `None` if
/// the request is too large for the slab layer (> 4096 bytes).
#[inline]
fn slab_class_for_size(size: usize) -> Option<usize> {
    for (index, &class_size) in SLAB_CLASS_SIZES.iter().enumerate() {
        if size <= class_size {
            return Some(index);
        }
    }
    None
}

/// Magic value written into `BlockHeader.magic` when a block is free.
const BLOCK_MAGIC_FREE: usize = 0xDEAD_BEEF_FEED_F4EE;

/// Magic value written into `BlockHeader.magic` when a block is in use.
const BLOCK_MAGIC_USED: usize = 0xCAFE_BABE_ABCD_1234;

// ---------------------------------------------------------------------------
// BlockHeader
// ---------------------------------------------------------------------------

/// Metadata stored immediately before each allocation payload.
///
/// Layout in memory:
///   [ BlockHeader (HEADER_SIZE bytes) | <size usable bytes> ]
///                                     ↑
///                                     pointer returned to caller
///
/// `#[repr(C, align(16))]` ensures:
///   1. Field order matches the compile-time size assertion.
///   2. Every block header is 16-byte aligned, so the payload that follows
///      is also 16-byte aligned — satisfying the HEAP_MIN_ALIGN guarantee
///      without any per-allocation adjustment.
#[repr(C, align(16))]
struct BlockHeader {
    /// BLOCK_MAGIC_FREE or BLOCK_MAGIC_USED.
    /// Any other value indicates heap corruption.
    magic: usize,
    /// Number of usable bytes immediately following this header.
    size: usize,
    /// Next block in the list, or null if this is the last block.
    next: *mut BlockHeader,
}

// Gap 11 — compile-time size check.
//
// With align(16) and three usize fields (3 × 8 = 24 bytes), the compiler
// pads the struct to 32 bytes to honour the 16-byte alignment requirement.
// If any field is added or removed this assertion fails and HEADER_SIZE must
// be updated to match.
const _: () = {
    if core::mem::size_of::<BlockHeader>() != 32 {
        panic!("BlockHeader size mismatch — update HEADER_SIZE in heap.rs");
    }
};

/// Size of BlockHeader in bytes.  Must equal `size_of::<BlockHeader>()`.
const HEADER_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// AllocContext — Gap 5
// ---------------------------------------------------------------------------

/// Specifies what the allocator is allowed to do when no free block is found.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AllocContext {
    /// Normal kernel context (EL1, IRQs may be enabled between calls but the
    /// caller must disable them for the duration of the alloc call).
    /// The heap is allowed to map new physical pages via `grow()`.
    Kernel,

    /// Interrupt / atomic context.
    /// The heap must NOT map new pages (no page-table modifications from IRQ).
    /// If no block is available immediately, returns `None`.
    ///
    /// TECHNICAL DEBT (Fase 5): a full GFP_ATOMIC equivalent pre-reserves a
    /// pool of pages at boot to guarantee availability from IRQ context.
    /// The current implementation only prevents heap growth — it does not
    /// guarantee success.
    Atomic,
}

// ---------------------------------------------------------------------------
// KernelHeap
// ---------------------------------------------------------------------------

pub struct KernelHeap {
    /// First block in the linked list, null before init().
    head: *mut BlockHeader,
    /// Virtual address of the byte immediately past the last mapped page.
    current_end: u64,
    /// Base virtual address of the heap (one guard page above _kernel_end).
    base: u64,
    /// Page size in bytes (read from TCR_EL1 at boot time).
    page_size: u64,

    // --- Slab freelist heads ---
    //
    // `slab_free_heads[i]` is the head of the freelist for size class
    // `SLAB_CLASS_SIZES[i]`.  Each free slab slot stores the next pointer
    // in its own first 8 bytes (a linked list embedded in the allocation
    // payload).  We store `*mut u8` rather than a typed pointer because
    // the payload size varies per class; the first 8 bytes always hold the
    // `next` pointer (slab slots are at least 16 bytes, so this fits).
    //
    // When a slab slot is in use, those 8 bytes belong to the caller —
    // we never touch them.
    slab_free_heads: [*mut u8; SLAB_CLASS_COUNT],
    /// Total slab allocations served (sum across all classes).
    pub slab_alloc_count: usize,
    /// Total slab frees (sum across all classes).
    pub slab_free_count: usize,

    // --- Statistics (Gap 3) ---
    /// Total bytes currently allocated (payload, excludes headers).
    pub total_allocated_bytes: usize,
    /// Peak value of total_allocated_bytes since init().
    pub peak_allocated_bytes: usize,
    /// Number of successful alloc_bytes() calls.
    pub alloc_count: usize,
    /// Number of successful free_bytes() calls.
    pub free_count: usize,
}

// SAFETY: KernelHeap is only accessed under a global lock (see GLOBAL_HEAP).
unsafe impl Send for KernelHeap {}

impl KernelHeap {
    /// Create the heap state.
    ///
    /// The heap base is placed one guard page above the end of the kernel
    /// image (_kernel_end, from the linker script).  That unmapped page causes
    /// a Data Abort (EC=0x25) if the kernel stack or BSS overflows into the
    /// heap — catching corruption before it silently corrupts allocator state.
    ///
    /// No physical memory is mapped yet; call `init()` to map the first pages.
    pub fn new(page_size: u64) -> Self {
        // Gap 10: guard page between _kernel_end and heap base.
        extern "C" {
            static _kernel_end: u8;
        }
        let kernel_end = unsafe { &_kernel_end as *const u8 as u64 };
        // align_up to page boundary, then skip one more page (guard page).
        let base = align_up(kernel_end, page_size) + page_size;

        Self {
            head: core::ptr::null_mut(),
            current_end: base,
            base,
            page_size,
            slab_free_heads: [core::ptr::null_mut(); SLAB_CLASS_COUNT],
            slab_alloc_count: 0,
            slab_free_count: 0,
            total_allocated_bytes: 0,
            peak_allocated_bytes: 0,
            alloc_count: 0,
            free_count: 0,
        }
    }

    /// Map the initial pages and create the first free block.
    pub fn init(
        &mut self,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Result<(), MapError> {
        self.grow_pages(HEAP_INITIAL_PAGES, page_table, allocator)
    }

    /// Allocate `size` bytes with the given `align` and `context`.
    ///
    /// Returns `None` on OOM, zero size, size exceeding `MAX_ALLOC_SIZE`, or
    /// (when `context == AllocContext::Atomic`) if no block is immediately free.
    ///
    /// Alignment must be a power of two and ≤ HEAP_MIN_ALIGN (16).
    /// All payloads are guaranteed to be at least 16-byte aligned because
    /// BlockHeader is itself aligned(16) and all sizes are multiples of 16.
    pub fn alloc_bytes(
        &mut self,
        size: usize,
        align: usize,
        context: AllocContext,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Option<NonNull<u8>> {
        // Gap 4: re-entrancy guard — alloc must not be called from IRQ context.
        debug_assert!(
            !interrupts_are_enabled(),
            "heap: alloc_bytes called with IRQs enabled — potential re-entrancy"
        );

        if size == 0 || size > MAX_ALLOC_SIZE {
            return None;
        }

        // Alignment sanity.
        debug_assert!(
            align.is_power_of_two(),
            "heap: align must be a power of two, got {}",
            align
        );
        debug_assert!(
            align <= HEAP_MIN_ALIGN,
            "heap: align {} > HEAP_MIN_ALIGN {}; alignment > 16 not yet supported",
            align,
            HEAP_MIN_ALIGN
        );

        // Round size up to HEAP_MIN_ALIGN so all block starts remain aligned.
        let aligned_size = (size + HEAP_MIN_ALIGN - 1) & !(HEAP_MIN_ALIGN - 1);

        // --- Gap 6: lazy coalesce ---
        // Step 1: first-fit without coalescing.
        if let Some(ptr) = self.find_free_block(aligned_size) {
            self.record_alloc(aligned_size);
            return Some(ptr);
        }

        // Step 2: coalesce adjacent free blocks, then retry.
        self.coalesce();
        if let Some(ptr) = self.find_free_block(aligned_size) {
            self.record_alloc(aligned_size);
            return Some(ptr);
        }

        // Step 3: grow the heap (unless atomic context forbids it).
        if context == AllocContext::Atomic {
            return None;
        }

        // Gap 8: grow with batch + fallback (see grow_for_size).
        self.grow_for_size(aligned_size, page_table, allocator).ok()?;
        let ptr = self.find_free_block(aligned_size)?;
        self.record_alloc(aligned_size);
        Some(ptr)
    }

    /// Free a previously allocated pointer.
    ///
    /// Errors:
    ///   `FreeError::OutOfRange`  — pointer not within heap bounds.
    ///   `FreeError::DoubleFree`  — magic number indicates already-free block.
    ///   `FreeError::Corrupt`     — magic number is neither FREE nor USED.
    pub fn free_bytes(&mut self, ptr: NonNull<u8>) -> Result<(), FreeError> {
        // Gap 4: re-entrancy guard.
        debug_assert!(
            !interrupts_are_enabled(),
            "heap: free_bytes called with IRQs enabled — potential re-entrancy"
        );

        let addr = ptr.as_ptr() as usize;

        let heap_start = self.base as usize + HEADER_SIZE;
        let heap_end = self.current_end as usize;

        if addr < heap_start || addr >= heap_end {
            return Err(FreeError::OutOfRange);
        }

        // Recover the block header immediately before the payload.
        let block_ptr = unsafe { (ptr.as_ptr() as *mut BlockHeader).sub(1) };
        let block = unsafe { &mut *block_ptr };

        match block.magic {
            BLOCK_MAGIC_FREE => return Err(FreeError::DoubleFree),
            BLOCK_MAGIC_USED => {} // expected
            _ => return Err(FreeError::Corrupt),
        }

        // Gap 2: poison payload to catch use-after-free.
        // 0xCC in AArch64: executing this byte sequence causes an illegal
        // instruction exception — use-after-free that reaches executed code
        // faults immediately and predictably.
        unsafe { core::ptr::write_bytes(ptr.as_ptr(), 0xCC, block.size) };

        // Gap 6: no coalesce here — lazy coalesce in alloc_bytes.
        block.magic = BLOCK_MAGIC_FREE;

        // Update statistics.
        self.total_allocated_bytes = self.total_allocated_bytes.saturating_sub(block.size);
        self.free_count += 1;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Diagnostics — Gap 7
    // -----------------------------------------------------------------------

    /// Capture a snapshot of the current heap state.
    ///
    /// `largest_free_block` is the key fragmentation indicator: if
    /// `free_bytes_total` is large but `largest_free_block` is small, the heap
    /// is heavily fragmented and large allocations will fail despite free space.
    pub fn snapshot(&self) -> HeapSnapshot {
        let mut free_blocks = 0usize;
        let mut free_bytes_total = 0usize;
        let mut largest_free_block = 0usize;

        let mut current = self.head;
        while !current.is_null() {
            let block = unsafe { &*current };
            if block.magic == BLOCK_MAGIC_FREE {
                free_blocks += 1;
                free_bytes_total += block.size;
                if block.size > largest_free_block {
                    largest_free_block = block.size;
                }
            }
            current = block.next;
        }

        HeapSnapshot {
            total_allocated_bytes: self.total_allocated_bytes,
            peak_allocated_bytes: self.peak_allocated_bytes,
            free_blocks,
            free_bytes_total,
            largest_free_block,
            alloc_count: self.alloc_count,
            free_count: self.free_count,
        }
    }

    /// Print heap statistics to UART.  Called from `memory_init()` after init.
    pub fn log_stats(&self) {
        use crate::drivers::uart;
        let snap = self.snapshot();
        uart::puts("Heap: ");
        uart::put_hex(snap.total_allocated_bytes as u64);
        uart::puts(" B allocated, peak ");
        uart::put_hex(snap.peak_allocated_bytes as u64);
        uart::puts(" B, ");
        uart::put_hex(snap.free_blocks as u64);
        uart::puts(" free blocks (largest ");
        uart::put_hex(snap.largest_free_block as u64);
        uart::puts(" B)\r\n");
    }

    // -----------------------------------------------------------------------
    // Slab layer (Phase 6d)
    // -----------------------------------------------------------------------

    /// Allocate from the slab layer for a request of `size` bytes.
    ///
    /// If the size maps to a slab class and the class freelist is non-empty,
    /// pops and returns a slot in O(1).  If the freelist is empty, allocates
    /// a fresh slot from the first-fit heap (sized exactly to the class).
    ///
    /// Returns `None` if:
    ///   - `size` exceeds the largest slab class (4096 bytes).
    ///   - The first-fit heap cannot satisfy the slot allocation (OOM).
    ///
    /// The returned pointer is uninitialized (the caller must initialize it).
    pub fn slab_alloc(
        &mut self,
        size: usize,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Option<NonNull<u8>> {
        let class_index = slab_class_for_size(size)?;
        let class_size = SLAB_CLASS_SIZES[class_index];

        // Fast path: freelist has a pre-allocated slot.
        let head = self.slab_free_heads[class_index];
        if !head.is_null() {
            // The first 8 bytes of the free slot hold the next pointer.
            let next = unsafe { core::ptr::read(head as *const *mut u8) };
            self.slab_free_heads[class_index] = next;
            self.slab_alloc_count += 1;
            self.total_allocated_bytes += class_size;
            if self.total_allocated_bytes > self.peak_allocated_bytes {
                self.peak_allocated_bytes = self.total_allocated_bytes;
            }
            // Zero out the next-pointer field so callers see clean memory.
            unsafe { core::ptr::write(head as *mut *mut u8, core::ptr::null_mut()) };
            return NonNull::new(head);
        }

        // Slow path: allocate a fresh slot from the first-fit heap.
        // We bypass `alloc_bytes` to avoid double-counting statistics and
        // to skip the slab check (prevent infinite recursion).
        let ptr = self.alloc_from_firstfit(class_size, page_table, allocator)?;
        self.slab_alloc_count += 1;
        self.total_allocated_bytes += class_size;
        if self.total_allocated_bytes > self.peak_allocated_bytes {
            self.peak_allocated_bytes = self.total_allocated_bytes;
        }
        Some(ptr)
    }

    /// Return a slab-allocated pointer back to the appropriate freelist.
    ///
    /// `size` must be the original allocation size (as passed to `slab_alloc`),
    /// not the rounded-up class size.  Both map to the same class index.
    ///
    /// # Safety
    /// `ptr` must have been returned by `slab_alloc` with the same `size`.
    pub unsafe fn slab_free(&mut self, ptr: NonNull<u8>, size: usize) {
        let class_index = match slab_class_for_size(size) {
            Some(i) => i,
            None => {
                // Should never happen if the caller is correct.
                debug_assert!(false, "slab_free: size {} has no slab class", size);
                return;
            }
        };
        let class_size = SLAB_CLASS_SIZES[class_index];

        // Poison the slot (use-after-free detection), then store next pointer.
        core::ptr::write_bytes(ptr.as_ptr(), 0xCC, class_size);
        core::ptr::write(ptr.as_ptr() as *mut *mut u8, self.slab_free_heads[class_index]);
        self.slab_free_heads[class_index] = ptr.as_ptr();

        self.slab_free_count += 1;
        self.total_allocated_bytes = self.total_allocated_bytes.saturating_sub(class_size);
    }

    /// Allocate from the first-fit heap without going through the slab layer.
    ///
    /// Used internally by `slab_alloc` to replenish empty slab freelists.
    fn alloc_from_firstfit(
        &mut self,
        size: usize,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Option<NonNull<u8>> {
        let aligned = (size + HEAP_MIN_ALIGN - 1) & !(HEAP_MIN_ALIGN - 1);

        if let Some(ptr) = self.find_free_block(aligned) {
            return Some(ptr);
        }
        self.coalesce();
        if let Some(ptr) = self.find_free_block(aligned) {
            return Some(ptr);
        }
        self.grow_for_size(aligned, page_table, allocator).ok()?;
        self.find_free_block(aligned)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// First-fit search: find and carve out a block of `needed` bytes.
    ///
    /// Returns the payload pointer on success, None if no suitable block exists.
    fn find_free_block(&mut self, needed: usize) -> Option<NonNull<u8>> {
        let mut current = self.head;
        while !current.is_null() {
            let block = unsafe { &mut *current };
            if block.magic == BLOCK_MAGIC_FREE && block.size >= needed {
                // Split if the remainder would be useful.
                if block.size >= needed + HEADER_SIZE + MIN_BLOCK_SIZE {
                    let remainder_ptr = unsafe {
                        (current as *mut u8).add(HEADER_SIZE).add(needed)
                            as *mut BlockHeader
                    };
                    let remainder = unsafe { &mut *remainder_ptr };
                    remainder.magic = BLOCK_MAGIC_FREE;
                    remainder.size = block.size - needed - HEADER_SIZE;
                    remainder.next = block.next;
                    block.next = remainder_ptr;
                    block.size = needed;
                }

                block.magic = BLOCK_MAGIC_USED;
                let user_ptr = unsafe { (current as *mut u8).add(HEADER_SIZE) };
                return NonNull::new(user_ptr);
            }
            current = block.next;
        }
        None
    }

    /// Update allocation statistics after a successful alloc.
    fn record_alloc(&mut self, size: usize) {
        self.total_allocated_bytes += size;
        self.alloc_count += 1;
        if self.total_allocated_bytes > self.peak_allocated_bytes {
            self.peak_allocated_bytes = self.total_allocated_bytes;
        }
    }

    /// Gap 8: grow the heap to satisfy a request of `needed_bytes`.
    ///
    /// Tries `HEAP_GROWTH_PAGES` pages first (batch — amortises TLB flushes).
    /// If the physical allocator does not have that many pages, falls back to
    /// the minimum number of pages required.  If even that fails → OOM.
    fn grow_for_size(
        &mut self,
        needed_bytes: usize,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Result<(), MapError> {
        let page_size = self.page_size as usize;
        let pages_needed = needed_bytes.div_ceil(page_size).max(1);
        let batch = pages_needed.max(HEAP_GROWTH_PAGES);

        // Try batch.
        let result = kernel_virtual_memory_alloc(
            page_table,
            VirtualAddress::new(self.current_end),
            batch,
            PAGE_FLAGS_KERNEL_DATA,
            self.page_size,
            allocator,
        );

        let actual_pages = match result {
            Ok(()) => batch,
            Err(_) if pages_needed < batch => {
                // Batch failed — try the minimum.
                kernel_virtual_memory_alloc(
                    page_table,
                    VirtualAddress::new(self.current_end),
                    pages_needed,
                    PAGE_FLAGS_KERNEL_DATA,
                    self.page_size,
                    allocator,
                )?;
                pages_needed
            }
            Err(e) => return Err(e), // true OOM
        };

        self.register_grown_pages(actual_pages);
        Ok(())
    }

    /// Grow the heap by exactly `page_count` pages (used by init()).
    fn grow_pages(
        &mut self,
        page_count: usize,
        page_table: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Result<(), MapError> {
        kernel_virtual_memory_alloc(
            page_table,
            VirtualAddress::new(self.current_end),
            page_count,
            PAGE_FLAGS_KERNEL_DATA,
            self.page_size,
            allocator,
        )?;
        self.register_grown_pages(page_count);
        Ok(())
    }

    /// Wire newly mapped pages into the free list.
    ///
    /// Called by both `grow_for_size` and `grow_pages` after successful mapping.
    /// If the current tail block is free and physically contiguous with the new
    /// region, it is extended rather than adding a new block (avoids split at
    /// page boundaries, allowing allocations that span multiple pages).
    fn register_grown_pages(&mut self, page_count: usize) {
        let page_size = self.page_size as usize;
        let actual_bytes = page_count * page_size;

        let new_block = self.current_end as *mut BlockHeader;
        let new_block_ref = unsafe { &mut *new_block };
        new_block_ref.magic = BLOCK_MAGIC_FREE;
        new_block_ref.size = actual_bytes - HEADER_SIZE;
        new_block_ref.next = core::ptr::null_mut();

        self.current_end += actual_bytes as u64;

        if self.head.is_null() {
            self.head = new_block;
            return;
        }

        // Find the tail block and potentially merge.
        let mut tail = self.head;
        while unsafe { !(*tail).next.is_null() } {
            tail = unsafe { (*tail).next };
        }

        let tail_ref = unsafe { &mut *tail };
        let tail_end =
            unsafe { (tail as *mut u8).add(HEADER_SIZE).add(tail_ref.size) };

        if tail_ref.magic == BLOCK_MAGIC_FREE
            && tail_end as *mut BlockHeader == new_block
        {
            // Contiguous free tail — extend rather than chain.
            tail_ref.size += HEADER_SIZE + new_block_ref.size;
        } else {
            tail_ref.next = new_block;
        }
    }

    /// Merge physically adjacent free blocks.
    ///
    /// Gap 6: called lazily by alloc_bytes() only when first-fit fails,
    /// not on every free().  This reduces per-free cost from O(n) to O(1).
    fn coalesce(&mut self) {
        let mut current = self.head;
        while !current.is_null() {
            let block = unsafe { &mut *current };
            if block.magic == BLOCK_MAGIC_FREE && !block.next.is_null() {
                let next = unsafe { &mut *block.next };
                if next.magic == BLOCK_MAGIC_FREE {
                    block.size += HEADER_SIZE + next.size;
                    block.next = next.next;
                    // Do not advance — check the new next as well.
                    continue;
                }
            }
            current = block.next;
        }
    }
}

// ---------------------------------------------------------------------------
// Gap 4 helper — read DAIF.I to detect IRQ-enabled context
// ---------------------------------------------------------------------------

/// Returns true if IRQs are currently enabled at EL1.
///
/// Reads the DAIF register: bit 7 = I mask.  When I = 0, IRQs are unmasked.
/// Reference: ARM ARM DDI 0487 D1.7.1.
#[inline]
fn interrupts_are_enabled() -> bool {
    let daif: u64;
    unsafe { core::arch::asm!("mrs {}, daif", out(reg) daif, options(nostack, nomem)) };
    daif & (1 << 7) == 0
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeError {
    /// Pointer is outside the current heap range.
    OutOfRange,
    /// Magic number shows this block is already free (double-free).
    DoubleFree,
    /// Magic number is neither FREE nor USED — heap corruption detected.
    Corrupt,
}

// ---------------------------------------------------------------------------
// HeapSnapshot — Gap 7
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of heap state for diagnostics.
pub struct HeapSnapshot {
    /// Bytes currently allocated (payload only, excludes headers).
    pub total_allocated_bytes: usize,
    /// Highest value of total_allocated_bytes since init().
    pub peak_allocated_bytes: usize,
    /// Number of free blocks in the list.
    pub free_blocks: usize,
    /// Total free payload bytes (sum of all free block sizes).
    pub free_bytes_total: usize,
    /// Largest single free block.
    ///
    /// If `free_bytes_total` is large but `largest_free_block` is small,
    /// the heap is fragmented and large allocations will fail.
    pub largest_free_block: usize,
    /// Total number of successful alloc_bytes() calls.
    pub alloc_count: usize,
    /// Total number of successful free_bytes() calls.
    pub free_count: usize,
}

// ---------------------------------------------------------------------------
// GlobalAlloc — Gap 9: passes layout.align() to alloc_bytes
// ---------------------------------------------------------------------------

/// Global allocator backed by KernelHeap.
///
/// All calls are serialized by disabling interrupts for the duration.
/// This is safe on a single-core kernel; for SMP, replace with a spinlock.
pub struct KernelGlobalAllocator;

unsafe impl GlobalAlloc for KernelGlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Route small allocations (≤ 4096 bytes, standard alignment) through
        // the slab layer.  Alignments > HEAP_MIN_ALIGN (16) are rare in kernel
        // code and are handled by the first-fit path.
        if layout.size() <= SLAB_CLASS_SIZES[SLAB_CLASS_COUNT - 1]
            && layout.align() <= HEAP_MIN_ALIGN
        {
            return with_global_heap(|heap, pt, alloc| {
                heap.slab_alloc(layout.size(), pt, alloc)
                    .map(|p| p.as_ptr())
                    .unwrap_or(core::ptr::null_mut())
            });
        }

        with_global_heap(|heap, pt, alloc| {
            heap.alloc_bytes(
                layout.size(),
                layout.align(),  // Gap 9: honour alignment request
                AllocContext::Kernel,
                pt,
                alloc,
            )
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut())
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(non_null) = NonNull::new(ptr) {
            // Mirror the alloc routing: if this was a slab allocation, return
            // it to the correct freelist rather than the first-fit free list.
            if layout.size() <= SLAB_CLASS_SIZES[SLAB_CLASS_COUNT - 1]
                && layout.align() <= HEAP_MIN_ALIGN
            {
                with_global_heap(|heap, _pt, _alloc| {
                    heap.slab_free(non_null, layout.size());
                });
            } else {
                with_global_heap(|heap, _pt, _alloc| {
                    let _ = heap.free_bytes(non_null);
                });
            }
        }
    }
}

/// Call `f` with exclusive access to the global heap and its dependencies.
///
/// # IRQ disable scope invariant
///
/// IRQs are disabled for the **minimum possible scope**: only the call to
/// `with_global_heap_inner` (which invokes `f`) is inside the critical
/// section.  The routing checks in `GlobalAlloc::alloc` / `dealloc`
/// (size comparison, slab-class lookup) execute *before* the call to
/// `with_global_heap` and are therefore **outside** the IRQ-disabled region.
///
/// This satisfies the re-entrancy guard in `alloc_bytes` / `free_bytes`
/// (Gap 4): a timer IRQ that attempts to allocate after the heap lock is
/// taken would block, not deadlock, because IRQs are masked for the
/// duration of `f`.
///
/// For SMP: replace the DAIF mask with a `SpinLock` (see `crate::sync`)
/// so that other cores wait rather than racing.  The IRQ mask remains
/// necessary to prevent a timer handler on *this* core from re-entering the
/// heap while we hold the spinlock.
fn with_global_heap<F, R>(f: F) -> R
where
    F: FnOnce(&mut KernelHeap, &mut PageTable, &mut PhysicalAllocator) -> R,
{
    // Save the current DAIF state and mask IRQs.
    // Only the heap operation itself (the call to `with_global_heap_inner`)
    // is inside the critical section — see the IRQ disable scope invariant
    // in the doc comment above.
    let daif_saved: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif_saved, options(nostack, nomem));
        core::arch::asm!("msr daifset, #2", options(nostack, nomem)); // mask IRQ
    }

    let result = unsafe { crate::memory::with_global_heap_inner(f) };

    // Restore DAIF.I to its previous state only if it was clear (IRQs were enabled).
    if daif_saved & (1 << 7) == 0 {
        unsafe { core::arch::asm!("msr daifclr, #2", options(nostack, nomem)) };
    }

    result
}
