#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/kernel.h"

// The free list stores virtual addresses (via HHDM), not physical addresses.
// This means nodes are always accessible regardless of which page table is active,
// as long as the HHDM is mapped — which is true from the moment hhdm_offset is set.
//
// physical_memory_alloc returns a PHYSICAL address (suitable for page table entries).
// physical_memory_free expects a PHYSICAL address.
// Callers use PHYSICAL_TO_VIRTUAL to access the memory contents if needed.
struct free_page {
    struct free_page *next;  // virtual address of next free page
};

static struct free_page *free_list_head = NULL;
static size_t free_page_count = 0;

void physical_memory_init(struct limine_memmap_response *memmap) {
    for (uint64_t i = 0; i < memmap->entry_count; i++) {
        struct limine_memmap_entry *entry = memmap->entries[i];

        if (entry->type != LIMINE_MEMMAP_USABLE) {
            continue;
        }

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
        return NULL;
    }

    // The node is stored at a virtual address — convert back to physical before returning.
    struct free_page *node = free_list_head;
    free_list_head = node->next;
    free_page_count--;

    return (void *)VIRTUAL_TO_PHYSICAL(node);
}

void physical_memory_free(void *physical_page) {
    // Store the node at the virtual address of the page so it remains accessible
    // after our own page table (which only maps via HHDM) is activated.
    struct free_page *node = (struct free_page *)PHYSICAL_TO_VIRTUAL(physical_page);
    node->next = free_list_head;
    free_list_head = node;
    free_page_count++;
}

size_t physical_memory_free_page_count(void) {
    return free_page_count;
}
