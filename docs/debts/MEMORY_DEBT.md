# Memory Subsystem — Pending

## Overview

The memory subsystem is functional but lacks demand paging and a few correctness
refinements. These items affect performance and correctness under real workloads.

---

## 1. Demand paging (lazy ELF segment loading)

### Current state
`loader/mod.rs` maps all ELF `PT_LOAD` segments eagerly: for every page in every
segment, a physical page is allocated and the segment data copied in. A binary
with a 100 MB `.bss` section consumes 100 MB of physical RAM at `exec()` time,
even if 90 MB of it is never touched.

### Required changes

**a. Lazy `.bss` mapping**

For ELF segments where `p_filesz < p_memsz`, the region `[p_vaddr + p_filesz,
p_vaddr + p_memsz)` is zero-filled BSS. Instead of allocating physical pages:
1. Map the BSS range as **not present** (PTE present bit = 0).
2. On the first write to any page in that range, a Data Abort (EC=0x24) fires.
3. The page fault handler (`memory::handle_page_fault`) detects the fault VA is in
   a known lazy-BSS mapping, allocates a zeroed page, maps it RW, and returns.

This requires a `VmArea` table per process tracking `(start_va, end_va, kind)` where
`kind` can be `LazyZero`, `CowFile(Arc<FileData>)`, or `Anonymous`.

**b. Lazy file-backed segments**

For executable segments (`PT_LOAD` with `PF_R | PF_X`), pages can be loaded from
the ELF image on demand. This requires:
- Keeping a reference to the ELF source (currently an embedded `&[u8]` slice).
- On page fault in the segment range: copy the 4 KB page from the ELF data.
- Read-only pages (code) can be shared across `fork()` / `exec()` without CoW.

**c. Integration with CoW fork**

The CoW fork (`scheduler::fork()` → `PageTable::cow_copy_user()`) already
marks pages read-only. The page fault handler already handles CoW copy. The
lazy-BSS path must integrate with this: a CoW fault on a lazy-BSS page should
allocate-and-zero (not copy from a source page).

### Files affected
- `loader/mod.rs` — skip physical allocation for BSS; record VmArea entries.
- `process/mod.rs` — add `vm_areas: Vec<VmArea>` to `Process`.
- `memory/virtual_memory.rs` — extend `handle_page_fault` to check `vm_areas`.
- `arch/arm64/exceptions/mod.rs` — already dispatches to `handle_page_fault`.

Reference: Linux `mm/memory.c` `do_anonymous_page()`, `do_fault()`.

---

## 2. `mmap MAP_SHARED` in fork()

### Current state
`MAP_SHARED | MAP_ANONYMOUS` regions are tracked in `SharedRegionTable` (a global
`Vec<SharedRegion>`). When `fork()` runs, it calls `cow_copy_user()` which marks
all RW pages as read-only for CoW. This incorrectly CoW-marks shared-memory pages
that should NOT be copied — both parent and child must see the same physical pages.

### Required change

In `cow_copy_user()` (or in `fork()` before calling it), identify pages that belong
to a `MAP_SHARED` region and skip the read-only re-mapping for those pages. Both
parent and child should point to the same physical page with the original RW
permissions.

Specifically:
1. In `scheduler::fork()`, after `cow_copy_user()`, re-walk the child's page table
   for each shared region VA range and restore the original RW mapping.
2. Or: pass the list of shared regions into `cow_copy_user()` and skip them there.

### Files affected
- `scheduler/mod.rs` — `fork()` must not CoW shared regions.
- `memory/virtual_memory.rs` — `cow_copy_user()` optionally receives exclusion list.

---

## 3. Physical memory allocator — buddy system

### Current state
`memory/physical.rs` uses a free-list allocator: freed pages are linked into a
`Vec`-based list. Allocation is O(n) in the number of free pages. Under heavy
allocation/deallocation workloads (many `fork()` + `exec()` cycles), the list
fragments and allocation becomes slow.

### Required change

Implement a binary buddy allocator:
- Maintain 12 free lists (orders 0–11, for 4 KB–16 MB blocks).
- `alloc(order)`: pop from the order-N list; if empty, split a block from order N+1.
- `free(addr, order)`: compute buddy address, merge if buddy is also free.
- Reduces fragmentation and gives O(log N) allocation.

### Files affected
- `memory/physical.rs` — replace `FreeList` with `BuddyAllocator`.

Reference: Knuth TAOCP §2.5, Linux `mm/page_alloc.c`.

---

## 4. Heap slab allocator — size class completeness

### Current state
`memory/heap.rs` uses size classes for allocations ≤ 4096 bytes (16, 32, 64, 128,
256, 512, 1024, 2048, 4096) and first-fit for larger. The slab implementation is
correct but the per-slab capacity is fixed at 64 objects regardless of object size.

### Required change

Compute per-slab capacity dynamically: `capacity = PAGE_SIZE / object_size`. For
16-byte objects that is 256 per slab; for 4096-byte objects that is 1. This
reduces internal fragmentation for large size classes.

### Files affected
- `memory/heap.rs` — `SlabCache::new()` capacity calculation.

---

## 5. Kernel stack guard pages

### Current state
Each process has a 64 KB kernel stack allocated by `KernelStack::allocate()`. There
is no guard page below the stack: a kernel stack overflow writes silently into
adjacent memory.

### Required change

In `KernelStack::allocate()`, after mapping the stack pages, call
`page_table.unmap(guard_va)` on the page immediately below the stack bottom. Any
stack overflow then faults at EL1 (Data Abort EC=0x25) rather than corrupting
adjacent memory.

Note: this requires `PageTable::unmap()` to accept a single VA (currently it exists
as `unmap_range`).

### Files affected
- `process/mod.rs` — `KernelStack::allocate()`.
- `memory/virtual_memory.rs` — expose single-page `unmap(va)` if not present.

---

## 6. ASLR entropy

### Current state
ASLR offsets are derived from `CNTPCT_EL0 ^ (page_table_root >> 12)` shifted to
give 0–65535 pages (256 MB range) for the stack, and the same for `mmap` base.
The entropy source is weak: `CNTPCT_EL0` is a timer that increments predictably
and an attacker with local access can measure it.

### Required change

Maintain a kernel entropy pool (`memory/entropy.rs`):
- Initialise with: `CNTPCT_EL0`, `CNTFRQ_EL0`, `MIDR_EL1`, PA of the initial
  stack pointer (read before the MMU was enabled), tick count at memory_init.
- Mix with a 64-bit LFSR (Galois form) after each extraction.
- Expose `entropy_get_u64() -> u64` used by the loader and mmap.

`getrandom` syscall (already number 29) should drain from this pool.

### Files affected
- New `memory/entropy.rs`.
- `loader/mod.rs` — use `entropy_get_u64()` for ASLR offset.
- `process/mod.rs` — use it for mmap base randomisation.
