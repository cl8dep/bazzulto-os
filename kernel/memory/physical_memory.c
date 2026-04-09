#include "../../include/bazzulto/physical_memory.h"

// Each free page stores a pointer to the next free page at its own address.
// This means the free list has zero memory overhead — the list nodes live
// inside the free pages themselves.
struct free_page {
    struct free_page *next;
};

static struct free_page *free_list_head = NULL;
static size_t free_page_count = 0;

void physical_memory_init(struct limine_memmap_response *memmap) {
    for (uint64_t i = 0; i < memmap->entry_count; i++) {
        struct limine_memmap_entry *entry = memmap->entries[i];

        // Only add pages from regions the firmware marked as usable.
        // Other types (reserved, ACPI, framebuffer, etc.) must not be touched.
        if (entry->type != LIMINE_MEMMAP_USABLE) {
            continue;
        }

        // Walk the region page by page and push each page onto the free list.
        // We cast the physical address to a pointer because at this stage the
        // kernel has a direct map of all physical memory (set up by Limine).
        uint64_t address = entry->base;
        uint64_t end     = entry->base + entry->length;

        while (address + PAGE_SIZE <= end) {
            physical_memory_free((void *)address);
            address += PAGE_SIZE;
        }
    }
}

void *physical_memory_alloc(void) {
    if (free_list_head == NULL) {
        return NULL;  // out of memory
    }

    // Pop the first page off the free list and return it.
    struct free_page *page = free_list_head;
    free_list_head = page->next;
    free_page_count--;

    return (void *)page;
}

void physical_memory_free(void *page) {
    // Write the current list head into the start of the page, then
    // make this page the new head. The page itself becomes the list node.
    struct free_page *node = (struct free_page *)page;
    node->next = free_list_head;
    free_list_head = node;
    free_page_count++;
}

size_t physical_memory_free_page_count(void) {
    return free_page_count;
}
