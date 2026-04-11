#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: Block Device (Disk I/O)
//
// Platform-independent interface for sector-level disk access.
// The platform backend (e.g. virtio-blk for QEMU virt) implements these.
// All operations use 512-byte sectors.
// ---------------------------------------------------------------------------

// Initialize the block device driver.
// Returns 0 on success, -1 if no block device is found.
int hal_disk_init(void);

// Return the interrupt ID registered for this block device.
// Returns 0 if no block device was found during init.
uint32_t hal_disk_get_irq_id(void);

// Handle the block device interrupt (e.g. I/O completion).
void hal_disk_irq_handler(void);

// Read `count` contiguous 512-byte sectors starting at `lba` into `buffer`.
// The buffer must hold at least count * 512 bytes.
// Blocks the calling process until the I/O completes.
// Returns 0 on success, -1 on error.
int hal_disk_read_sectors(uint64_t lba, uint32_t count, void *buffer);

// Write `count` contiguous 512-byte sectors from `buffer` starting at `lba`.
// The buffer must hold at least count * 512 bytes.
// Blocks the calling process until the I/O completes.
// Returns 0 on success, -1 on error.
int hal_disk_write_sectors(uint64_t lba, uint32_t count, const void *buffer);

// Return the total capacity in 512-byte sectors.
// Returns 0 if no block device is initialized.
uint64_t hal_disk_capacity(void);
