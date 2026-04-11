#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: Virtio-MMIO Bus Enumeration
//
// Platform-independent interface for discovering virtio devices.
// The platform backend provides the MMIO base address and slot layout.
// ---------------------------------------------------------------------------

// Scan all virtio-mmio slots and cache discovered device metadata.
// Must be called after MMIO pages are mapped.
void hal_virtio_enumerate(void);

// Find the first virtio device matching the given device_id.
// Writes the slot index to *slot_out (may be NULL).
// Returns the physical base address of the device, or 0 if not found.
uint64_t hal_virtio_find_device(int device_id, int *slot_out);
