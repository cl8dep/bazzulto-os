#pragma once

#include <stdint.h>

// ARM64 Stage 1 page descriptor flags
// Reference: ARM ARM DDI 0487 D5.3.3, Table D5-34

// Bits [1:0]: descriptor type
#define PAGE_VALID         (1ULL << 0)   // Bit 0: entry is valid/present
#define PAGE_TABLE         (1ULL << 1)   // Bit 1: page descriptor (L3) or table pointer (L0-L2)

// Bits [4:2]: AttrIdx — selects memory attributes from MAIR_EL1
// ARM ARM D5.5.1: AttrIdx indexes into the 8-byte MAIR register
#define PAGE_ATTR_NORMAL   (0ULL << 2)   // AttrIdx=0 → MAIR byte 0: normal WB cacheable (0xFF)
#define PAGE_ATTR_DEVICE   (1ULL << 2)   // AttrIdx=1 → MAIR byte 1: device nGnRnE (0x00)

// Bits [7:6]: AP (Access Permissions) — ARM ARM D5.4.4, Table D5-39
// AP[2] = bit 7, AP[1] = bit 6
#define PAGE_KERNEL_RW     (0ULL << 6)   // AP[2:1]=00: EL1 R/W, EL0 no access

// Bits [9:8]: SH (Shareability) — ARM ARM D5.5.3
// Must match TCR_EL1 shareability for correct cache behavior
#define PAGE_SH_NON        (0ULL << 8)   // Non-shareable
#define PAGE_SH_OUTER      (2ULL << 8)   // Outer Shareable
#define PAGE_SH_INNER      (3ULL << 8)   // Inner Shareable

// Bit [10]: AF (Access Flag) — ARM ARM D5.4.5
#define PAGE_ACCESS_FLAG   (1ULL << 10)  // Must be 1 or CPU raises Access Flag fault

// Bit [53]: PXN (Privileged Execute Never) — ARM ARM D5.4.5
// For EL1&0 stage 1: prevents EL1 from executing this page
#define PAGE_PXN           (1ULL << 53)

// Bit [54]: UXN (User Execute Never) — ARM ARM D5.4.5
// For EL1&0 stage 1: prevents EL0 from executing this page
#define PAGE_UXN           (1ULL << 54)

// Bits [7:6]: AP for user-accessible pages — ARM ARM D5.4.4, Table D5-39
#define PAGE_USER_RW       (1ULL << 6)   // AP[2:1]=01: EL1+EL0 read/write
#define PAGE_USER_RO       (3ULL << 6)   // AP[2:1]=11: EL1+EL0 read-only

// --- Composite flags for common mappings ---

// Kernel code (.text): executable by EL1, not by EL0
#define PAGE_FLAGS_KERNEL_CODE \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_KERNEL_RW | \
     PAGE_ATTR_NORMAL | PAGE_SH_INNER | PAGE_UXN)

// Kernel data (.rodata, .data, .bss, heap, stacks): not executable by anyone
#define PAGE_FLAGS_KERNEL_DATA \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_KERNEL_RW | \
     PAGE_ATTR_NORMAL | PAGE_SH_INNER | PAGE_PXN | PAGE_UXN)

// Device MMIO (GIC, framebuffer, UART): not executable, device-nGnRnE
// ARM ARM B2.7.2: device memory is always treated as Outer Shareable
// regardless of SH bits, but setting SH=Outer is conventional.
#define PAGE_FLAGS_KERNEL_DEVICE \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_KERNEL_RW | \
     PAGE_ATTR_DEVICE | PAGE_SH_OUTER | PAGE_PXN | PAGE_UXN)

// User code (.text): executable by EL0, not by EL1. Read-only from both.
#define PAGE_FLAGS_USER_CODE \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_USER_RO | \
     PAGE_ATTR_NORMAL | PAGE_SH_INNER | PAGE_PXN)

// User data (stack, heap): read-write, not executable by anyone.
#define PAGE_FLAGS_USER_DATA \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_USER_RW | \
     PAGE_ATTR_NORMAL | PAGE_SH_INNER | PAGE_PXN | PAGE_UXN)

// Create a new, empty page table. Returns a virtual address pointer to it.
uint64_t *virtual_memory_create_table(void);

// Allocate n_pages physical pages and map them contiguously starting at vaddr
// in the kernel page table (kernel_page_table) with the given flags.
// Returns 0 on success, -1 if any physical allocation fails.
int kernel_vm_alloc(uint64_t vaddr, uint64_t n_pages, uint64_t flags);

// Unmap a contiguous range of n_pages pages starting at vaddr in the given
// page table, freeing the underlying physical pages. TLB entries are
// invalidated for each page. Table intermediate pages are NOT freed.
void virtual_memory_unmap_range(uint64_t *page_table,
                                uint64_t vaddr, uint64_t n_pages);

// Map a virtual address to a physical address in the given page table.
// Intermediate table levels are allocated automatically via physical_memory_alloc.
void virtual_memory_map(uint64_t *table, uint64_t virtual_addr, uint64_t physical_addr, uint64_t flags);

// Activate a page table by writing it into TTBR1_EL1 (kernel address space).
// After this call, the CPU uses this table for all 0xFFFF... addresses.
void virtual_memory_activate(uint64_t *kernel_table);

// Enable TTBR0 page table walks in TCR_EL1 (clears EPD0).
// Must be called once before any user process runs.
void virtual_memory_enable_user(void);

// Switch the user-space page table (TTBR0_EL1) to a new process's table.
// Flushes the TLB to ensure the new mappings take effect.
void virtual_memory_switch_ttbr0(uint64_t *user_table);

// Deep-copy a user page table: allocate fresh intermediate tables and fresh
// physical pages for every L3 leaf in src_table. Each leaf page is copied
// byte-for-byte. Only the lower-half VA range (TTBR0, < 2^48) is processed.
// Returns a pointer to the new L0 table (HHDM virtual address), or NULL on
// failure. On failure the partial copy leaks physical pages (no rollback).
uint64_t *virtual_memory_deep_copy_table(const uint64_t *src_table);
