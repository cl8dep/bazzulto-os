#pragma once

#include <stdint.h>
#include <stddef.h>
#include "../../limine/limine.h"

#define PAGE_SIZE 4096

// Initialize the physical memory allocator using the memory map provided by Limine.
// Must be called before any physical_memory_alloc calls.
void physical_memory_init(struct limine_memmap_response *memmap);

// Allocate one physical page (4KB). Returns the physical address, or NULL if
// no free pages remain.
void *physical_memory_alloc(void);

// Return a previously allocated page to the free list.
void physical_memory_free(void *page);

// Return the number of free pages currently available.
size_t physical_memory_free_page_count(void);

// Return the physical address ceiling of detected RAM — max(base+length) across
// all firmware memory map entries regardless of type. This equals the total
// installed RAM size as reported by the firmware (same approach as Linux e820).
// Use this to size kernel structures (e.g. PID bitmap, page frame arrays).
uint64_t physical_memory_total_bytes(void);

// Return the total number of usable bytes — sum of LIMINE_MEMMAP_USABLE entry
// lengths. Use this to report available RAM to users or calculate process limits.
uint64_t physical_memory_usable_bytes(void);
