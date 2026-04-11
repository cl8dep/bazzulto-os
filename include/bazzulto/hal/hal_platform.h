#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: Platform Bootstrap
//
// Each platform provides an MMIO region table (for page table mapping)
// and a post-heap initialization hook.
// ---------------------------------------------------------------------------

// Describes one MMIO region the platform needs mapped as device memory.
typedef struct {
    uint64_t physical_base;
    uint64_t size;  // in bytes (will be rounded up to page boundaries)
} hal_mmio_region_t;

// Return a NULL-terminated array of MMIO regions the platform requires.
// Called by main.c before activating the page table, so that all device
// registers are accessible via HHDM.
const hal_mmio_region_t *hal_platform_mmio_regions(void);

// Called after the heap and basic kernel services are ready.
// The platform performs remaining early initialization here
// (e.g. virtio bus enumeration).
void hal_platform_init(void);
