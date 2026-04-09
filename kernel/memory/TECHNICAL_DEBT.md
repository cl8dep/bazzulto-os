# Technical Debt — Memory Subsystem

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
