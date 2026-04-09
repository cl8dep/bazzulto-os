#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/kernel.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/virtual_memory.h"

// Every allocation is preceded by this header in memory.
// The header itself is not visible to the caller — kmalloc returns the
// address immediately after it.
struct block_header {
    size_t size;                // usable bytes in this block (excluding header)
    int is_free;                // 1 if available for allocation, 0 if in use
    struct block_header *next;  // next block in the heap (free or not)
};

#define HEADER_SIZE sizeof(struct block_header)

// Minimum useful block size. Splitting a block smaller than this wastes
// more memory on headers than it saves.
#define MIN_BLOCK_SIZE 32

// The heap is a contiguous region of virtual memory. We expand it by
// allocating physical pages and mapping them as needed.
// Starting address chosen to be well above the kernel image.
#define HEAP_START 0xFFFFFFFF90000000ULL
#define HEAP_MAX   (HEAP_START + (64ULL * 1024 * 1024))  // 64MB initial limit

static struct block_header *heap_head = NULL;
static uint64_t heap_current_end = HEAP_START;

// Extend the heap by one page, adding it as a free block at the end.
static int heap_grow(void) {
    if (heap_current_end + PAGE_SIZE > HEAP_MAX) {
        return 0;  // heap limit reached
    }

    void *physical_page = physical_memory_alloc();
    if (!physical_page) return 0;

    // Map the new page into the heap's virtual address range.
    virtual_memory_map(kernel_page_table,
                       heap_current_end,
                       (uint64_t)physical_page,
                       PAGE_FLAGS_KERNEL_DATA);

    // Place a block header at the start of the new page.
    struct block_header *new_block = (struct block_header *)heap_current_end;
    new_block->size    = PAGE_SIZE - HEADER_SIZE;
    new_block->is_free = 1;
    new_block->next    = NULL;

    heap_current_end += PAGE_SIZE;

    // Append to the end of the block list.
    if (!heap_head) {
        heap_head = new_block;
    } else {
        struct block_header *current = heap_head;
        while (current->next) current = current->next;
        current->next = new_block;
    }

    return 1;
}

void heap_init(void) {
    // Allocate the first page to bootstrap the heap.
    heap_grow();
}

void *kmalloc(size_t size) {
    if (size == 0) return NULL;

    // Align size to 8 bytes so all allocations are naturally aligned.
    size = (size + 7) & ~(size_t)7;

    // First-fit search: find the first free block large enough.
    struct block_header *current = heap_head;
    while (current) {
        if (current->is_free && current->size >= size) {
            // Split the block if the leftover would be useful.
            if (current->size >= size + HEADER_SIZE + MIN_BLOCK_SIZE) {
                struct block_header *remainder =
                    (struct block_header *)((uint8_t *)current + HEADER_SIZE + size);
                remainder->size    = current->size - size - HEADER_SIZE;
                remainder->is_free = 1;
                remainder->next    = current->next;
                current->next = remainder;
                current->size = size;
            }

            current->is_free = 0;
            return (void *)((uint8_t *)current + HEADER_SIZE);
        }
        current = current->next;
    }

    // No suitable block found — grow the heap and try once more.
    if (heap_grow()) return kmalloc(size);

    return NULL;  // out of memory
}

void kfree(void *ptr) {
    if (!ptr) return;

    struct block_header *block =
        (struct block_header *)((uint8_t *)ptr - HEADER_SIZE);
    block->is_free = 1;

    // Coalesce adjacent free blocks to prevent fragmentation.
    // Walk from the head and merge any two consecutive free blocks.
    struct block_header *current = heap_head;
    while (current && current->next) {
        if (current->is_free && current->next->is_free) {
            current->size += HEADER_SIZE + current->next->size;
            current->next  = current->next->next;
        } else {
            current = current->next;
        }
    }
}
