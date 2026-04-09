# Memory Subsystem

## Overview

The memory subsystem is split into two layers that must be initialized in order:

1. **Physical Memory Allocator** — manages raw 4KB pages of physical RAM
2. **Virtual Memory Manager** — builds page tables and activates the MMU

---

## Physical Memory Allocator (`physical_memory.c`)

Implements a **free list** allocator. Each free page stores a pointer to the
next free page at its own address — the list has zero memory overhead because
the nodes live inside the free pages themselves.

### API

```c
void  physical_memory_init(struct limine_memmap_response *memmap);
void *physical_memory_alloc(void);   // returns a physical address, or NULL
void  physical_memory_free(void *page);
size_t physical_memory_free_page_count(void);
```

### Initialization

Iterates every entry in the Limine memory map. Only regions marked
`LIMINE_MEMMAP_USABLE` are added to the free list — reserved, ACPI, framebuffer,
and other regions must not be touched.

### Known limitation

The free list suffers from **external fragmentation**: freed pages scatter across
physical memory over time. Allocating N *contiguous* pages may fail even when
enough total free memory exists. See `TECHNICAL_DEBT.md` for the migration path
to a Buddy System allocator.

---

## Virtual Memory Manager (`virtual_memory.c`)

Manages ARM64 4-level page tables and the MMU.

### Address space layout

ARM64 splits the 64-bit virtual address space into two halves based on bit 63:

| Range | Register | Purpose |
|---|---|---|
| `0x0000...` – `0x0000FFFFFFFFFFFF` | `TTBR0_EL1` | User space (per-process) |
| `0xFFFF000000000000` – `0xFFFFFFFFFFFFFFFF` | `TTBR1_EL1` | Kernel space (global) |

Bazzulto currently only sets up `TTBR1_EL1` (kernel space). User space tables
will be created per-process when the scheduler is implemented.

### Page table structure

Each level has 512 entries of 8 bytes. Each entry either points to the next
level table (bits 0–1 = `0b11`) or maps a physical page (level 3 only).

```
Virtual address (48 bits used):
  [47:39] → L0 index (512 entries)
  [38:30] → L1 index (512 entries)
  [29:21] → L2 index (512 entries)
  [20:12] → L3 index (512 entries)
  [11: 0] → byte offset within the 4KB page
```

Intermediate table nodes are allocated on demand from `physical_memory_alloc`.

### HHDM (Higher Half Direct Map)

All physical memory is mapped at `hhdm_offset + physical_address`. This allows
the kernel to read/write any physical address after the MMU is active:

```c
// Convert between physical and virtual addresses
PHYSICAL_TO_VIRTUAL(physical)  →  (void *)(physical + hhdm_offset)
VIRTUAL_TO_PHYSICAL(virtual)   →  (uint64_t)(virtual - hhdm_offset)
```

The HHDM is built by iterating the Limine memory map and mapping every physical
region — not a hardcoded size. This scales to any amount of RAM.

### Activation

`virtual_memory_activate()` configures three ARM64 system registers before
switching the page table:

- **`MAIR_EL1`** — defines memory attribute types (normal cacheable, device)
- **`TCR_EL1`** — sets address space size (48-bit) and cache policy
- **`TTBR1_EL1`** — loads the physical address of the kernel page table

After writing these registers, the TLB is flushed (`tlbi vmalle1`) and memory
barriers (`dsb sy`, `isb`) ensure the CPU sees the new state before executing
the next instruction.

### Address space capacity

With 48-bit virtual addresses, the kernel half (`TTBR1`) covers 128 TB of
virtual space — sufficient to HHDM-map hundreds of TB of physical RAM.
