#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// virtio-mmio register interface — virtio spec 1.1, section 4.2.2
//
// On QEMU virt, 32 virtio-mmio slots are mapped at 0x0A000000, each
// occupying 0x200 bytes. The IRQ for slot N is GIC SPI (16+N),
// which is INTID (48+N).
// Source: QEMU hw/arm/virt.c
// ---------------------------------------------------------------------------

#define VIRTIO_MMIO_BASE          0x0A000000ULL  // physical base of slot 0
#define VIRTIO_MMIO_SLOT_SIZE     0x200          // bytes per slot
#define VIRTIO_MMIO_NUM_SLOTS     32             // total slots on QEMU virt

// virtio-mmio register offsets — virtio spec 1.1 §4.2.2
#define VIRTIO_MMIO_MAGIC_VALUE           0x000  // R:  must be 0x74726976 ("virt")
#define VIRTIO_MMIO_VERSION               0x004  // R:  1 = legacy, 2 = modern (non-legacy)
#define VIRTIO_MMIO_DEVICE_ID             0x008  // R:  device type (18 = input)
#define VIRTIO_MMIO_VENDOR_ID             0x00C  // R:  vendor identifier
#define VIRTIO_MMIO_DEVICE_FEATURES       0x010  // R:  device feature bits (selected by DEVICE_FEATURES_SEL)
#define VIRTIO_MMIO_DEVICE_FEATURES_SEL   0x014  // W:  select feature word (0 = bits 0-31, 1 = bits 32-63)
#define VIRTIO_MMIO_DRIVER_FEATURES       0x020  // W:  driver feature bits acknowledged
#define VIRTIO_MMIO_DRIVER_FEATURES_SEL   0x024  // W:  select feature word
#define VIRTIO_MMIO_QUEUE_SEL             0x030  // W:  select virtqueue index
#define VIRTIO_MMIO_QUEUE_NUM_MAX         0x034  // R:  max descriptors for selected queue
#define VIRTIO_MMIO_QUEUE_NUM             0x038  // W:  actual descriptor count to use
#define VIRTIO_MMIO_QUEUE_READY           0x044  // RW: 1 = queue active
#define VIRTIO_MMIO_QUEUE_NOTIFY          0x050  // W:  notify device of available descriptors
#define VIRTIO_MMIO_INTERRUPT_STATUS      0x060  // R:  pending interrupt reasons
#define VIRTIO_MMIO_INTERRUPT_ACK         0x064  // W:  acknowledge and clear interrupts
#define VIRTIO_MMIO_STATUS                0x070  // RW: device status register
#define VIRTIO_MMIO_QUEUE_DESC_LOW        0x080  // W:  low 32 bits of descriptor table PA
#define VIRTIO_MMIO_QUEUE_DESC_HIGH       0x084  // W:  high 32 bits of descriptor table PA
#define VIRTIO_MMIO_QUEUE_DRIVER_LOW      0x090  // W:  low 32 bits of available ring PA
#define VIRTIO_MMIO_QUEUE_DRIVER_HIGH     0x094  // W:  high 32 bits of available ring PA
#define VIRTIO_MMIO_QUEUE_DEVICE_LOW      0x0A0  // W:  low 32 bits of used ring PA
#define VIRTIO_MMIO_QUEUE_DEVICE_HIGH     0x0A4  // W:  high 32 bits of used ring PA

// VIRTIO_MMIO_MAGIC_VALUE field — virtio spec 1.1 §4.2.2.2
#define VIRTIO_MMIO_MAGIC  0x74726976U  // little-endian "virt"

// virtio device IDs — virtio spec 1.1, Appendix B
#define VIRTIO_DEVICE_ID_INPUT  18

// virtio device status bits — virtio spec 1.1 §2.1
#define VIRTIO_DEVICE_STATUS_ACKNOWLEDGE  (1U << 0)
#define VIRTIO_DEVICE_STATUS_DRIVER       (1U << 1)
#define VIRTIO_DEVICE_STATUS_DRIVER_OK    (1U << 2)
#define VIRTIO_DEVICE_STATUS_FEATURES_OK  (1U << 3)
#define VIRTIO_DEVICE_STATUS_NEEDS_RESET  (1U << 6)
#define VIRTIO_DEVICE_STATUS_FAILED       (1U << 7)

// virtqueue descriptor flags — virtio spec 1.1 §2.7.5
#define VIRTQ_DESC_FLAG_NEXT   (1U << 0)  // descriptor chains to next
#define VIRTQ_DESC_FLAG_WRITE  (1U << 1)  // device writes into this buffer

// VIRTIO_MMIO_INTERRUPT_STATUS bits — virtio spec 1.1 §4.2.2.3
#define VIRTIO_MMIO_INT_VRING   (1U << 0)  // used ring updated
#define VIRTIO_MMIO_INT_CONFIG  (1U << 1)  // config space changed

// ---------------------------------------------------------------------------
// Legacy (version 1) register offsets — virtio spec 1.0, Appendix D
// These replace the modern DESC/DRIVER/DEVICE pointers with a single QueuePFN.
// The layout of the virtqueue (descriptor table + available ring + used ring)
// must reside in one contiguous block; QueuePFN gives the start (in pages).
// ---------------------------------------------------------------------------
#define VIRTIO_MMIO_LEGACY_HOST_FEATURES     0x010  // R:  device features (bits 0-31)
#define VIRTIO_MMIO_LEGACY_GUEST_FEATURES    0x020  // W:  driver features accepted
#define VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE   0x028  // W:  page size (must be 4096)
#define VIRTIO_MMIO_LEGACY_QUEUE_ALIGN       0x03C  // W:  virtqueue used-ring alignment
#define VIRTIO_MMIO_LEGACY_QUEUE_PFN         0x040  // RW: virtqueue base addr >> PAGE_SHIFT

// ---------------------------------------------------------------------------
// Enumeration API
// ---------------------------------------------------------------------------

// Scan all 32 virtio-mmio slots. Records physical base addresses and device
// IDs for all valid, modern (version 2) devices found.
// Must be called after the HHDM is active and virtio MMIO pages are mapped.
void virtio_mmio_enumerate(void);

// Return the physical base address of the first device matching device_id,
// and write its slot index to *slot_out (may be NULL if not needed).
// Returns 0 if no matching device was found.
uint64_t virtio_mmio_find_device(int device_id, int *slot_out);
