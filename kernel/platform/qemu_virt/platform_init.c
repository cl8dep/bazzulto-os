// QEMU virt platform initialization
//
// Provides the MMIO region table for page table setup and
// post-heap platform initialization.

#include "../../../include/bazzulto/hal/hal_platform.h"
#include "../../../include/bazzulto/hal/hal_virtio.h"

// QEMU virt MMIO regions — verified from DTB dump.
// These must be mapped as device memory before any driver initialization.
static const hal_mmio_region_t qemu_virt_mmio_regions[] = {
    { 0x08000000ULL, 0x20000 },  // GIC Distributor + CPU Interface (128 KB)
    { 0x09000000ULL, 0x1000  },  // PL011 UART0 (4 KB)
    { 0x0A000000ULL, 0x4000  },  // virtio-mmio slots 0-31 (16 KB)
    { 0, 0 }                     // NULL terminator
};

const hal_mmio_region_t *hal_platform_mmio_regions(void)
{
    return qemu_virt_mmio_regions;
}

void hal_platform_init(void)
{
    // Enumerate virtio-mmio bus — discovers keyboard, block devices, etc.
    hal_virtio_enumerate();
}
