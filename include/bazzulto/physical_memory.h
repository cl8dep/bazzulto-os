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
