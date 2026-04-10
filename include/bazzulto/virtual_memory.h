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

// Create a new, empty page table. Returns a virtual address pointer to it.
uint64_t *virtual_memory_create_table(void);

// Map a virtual address to a physical address in the given page table.
// Intermediate table levels are allocated automatically via physical_memory_alloc.
void virtual_memory_map(uint64_t *table, uint64_t virtual_addr, uint64_t physical_addr, uint64_t flags);

// Activate a page table by writing it into TTBR1_EL1 (kernel address space).
// After this call, the CPU uses this table for all 0xFFFF... addresses.
void virtual_memory_activate(uint64_t *kernel_table);
