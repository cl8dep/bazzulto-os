#pragma once

#include <stdint.h>

// ARM64 page table entry flags
#define PAGE_VALID         (1ULL << 0)   // entry is present
#define PAGE_TABLE         (1ULL << 1)   // points to next-level table (vs block)
#define PAGE_ACCESS_FLAG   (1ULL << 10)  // must be set or CPU raises Access Flag fault
#define PAGE_KERNEL_RW     (0ULL << 6)   // AP[1]=0: kernel read/write, user no access
#define PAGE_EXECUTE_NEVER (1ULL << 54)  // page is not executable
// AttrIdx=0 in bits [4:2]: references MAIR index 0 = normal write-back cacheable memory
#define PAGE_ATTR_NORMAL   (0ULL << 2)

// Flags for normal kernel data pages
#define PAGE_FLAGS_KERNEL_DATA \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_KERNEL_RW | PAGE_EXECUTE_NEVER | PAGE_ATTR_NORMAL)

// Flags for kernel code pages (executable)
#define PAGE_FLAGS_KERNEL_CODE \
    (PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG | PAGE_KERNEL_RW | PAGE_ATTR_NORMAL)

// Create a new, empty page table. Returns a virtual address pointer to it.
uint64_t *virtual_memory_create_table(void);

// Map a virtual address to a physical address in the given page table.
// Intermediate table levels are allocated automatically via physical_memory_alloc.
void virtual_memory_map(uint64_t *table, uint64_t virtual_addr, uint64_t physical_addr, uint64_t flags);

// Activate a page table by writing it into TTBR1_EL1 (kernel address space).
// After this call, the CPU uses this table for all 0xFFFF... addresses.
void virtual_memory_activate(uint64_t *kernel_table);
