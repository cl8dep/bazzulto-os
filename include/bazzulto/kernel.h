#pragma once

#include <stdint.h>

// The Higher Half Direct Map offset provided by Limine.
// All physical addresses must be offset by this value to obtain a valid
// virtual address the kernel can read/write.
//
// Example: physical 0x50000 → virtual (hhdm_offset + 0x50000)
//
// Set once during boot by kernel_main, then read-only forever after.
extern uint64_t hhdm_offset;

// Convert a physical address to a kernel virtual address via the HHDM.
#define PHYSICAL_TO_VIRTUAL(physical) ((void *)((uint64_t)(physical) + hhdm_offset))

// Convert a kernel virtual address back to a physical address.
#define VIRTUAL_TO_PHYSICAL(virtual)  ((uint64_t)(virtual) - hhdm_offset)
