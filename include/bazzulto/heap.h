#pragma once

#include <stddef.h>

// Initialize the kernel heap. Must be called after virtual memory is active.
void heap_init(void);

// Allocate at least `size` bytes from the kernel heap.
// Returns NULL if the heap is full or size is 0.
void *kmalloc(size_t size);

// Release a previously allocated block back to the heap.
// Passing NULL is safe and does nothing.
void kfree(void *ptr);
