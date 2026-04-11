# Technical Debt — Memory Subsystem

## Heap Double-Free Protection — DONE
`kfree` now panics on double-free (`console_println` + halt loop).

## Heap Abstraction — DONE
`heap_grow` calls `kernel_vm_alloc(vaddr, 1, flags)` instead of calling
`physical_memory_alloc` + `virtual_memory_map` directly.

## Heap Size — OPEN
`HEAP_MAX` is fixed at 64 MB from `0xFFFFFFFF90000000`. Dynamic growth up to
a configurable fraction of physical RAM is not yet implemented.

## PMM: Free List → Buddy System — OPEN
Physical memory allocator is still a simple free list. The buddy system is needed
to prevent external fragmentation under heavy load (many processes, large mmap
allocations). Migrate after the single-core scheduler is stable and fragmentation
becomes measurable.
