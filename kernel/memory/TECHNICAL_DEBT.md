# Technical Debt — Memory Subsystem

## Heap: no double-free protection

`kfree` does not verify that the block is actually in use before freeing it.
Calling `kfree(ptr)` twice silently corrupts the heap's free list, leading to
undefined behavior on the next allocation. Fix: add a check that `is_free == 0`
before marking a block free, and panic if the invariant is violated.

## Heap depends directly on virtual_memory_map

`heap_grow` calls `virtual_memory_map` directly to map new pages. This creates
a tight coupling between the heap and the virtual memory implementation. A cleaner
design would introduce a `kernel_vm_alloc(n_pages)` abstraction that the heap
calls, hiding the page table details.

---

## PMM: Free List → Buddy System

**Current implementation:** simple free list (singly linked list of free 4KB pages).

**Problem it will cause:** external fragmentation. Over time, freed pages scatter
across physical memory. When a caller needs N contiguous pages, the allocator may
fail even if enough total free memory exists — it just isn't contiguous.

**Proper solution:** Buddy System allocator, as used by Linux and FreeBSD.
Maintains separate free lists per power-of-two block order (1, 2, 4, 8 ... pages).
On free, merges adjacent buddies upward — prevents fragmentation structurally.

**When to migrate:** once processes and virtual memory are working and the
limitations of the free list become observable (allocation failures under load,
inability to satisfy large contiguous requests).
