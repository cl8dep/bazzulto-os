#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// Virtio specification constants — virtio spec 1.1
//
// These are protocol-defined values that are the same on every platform.
// Platform-specific details (MMIO base address, slot count, IRQ routing)
// live in the platform backend, not here.
// ---------------------------------------------------------------------------

// virtio-mmio register offsets — virtio spec 1.1 §4.2.2
#define VIRTIO_MMIO_MAGIC_VALUE           0x000
#define VIRTIO_MMIO_VERSION               0x004
#define VIRTIO_MMIO_DEVICE_ID             0x008
#define VIRTIO_MMIO_VENDOR_ID             0x00C
#define VIRTIO_MMIO_DEVICE_FEATURES       0x010
#define VIRTIO_MMIO_DEVICE_FEATURES_SEL   0x014
#define VIRTIO_MMIO_DRIVER_FEATURES       0x020
#define VIRTIO_MMIO_DRIVER_FEATURES_SEL   0x024
#define VIRTIO_MMIO_QUEUE_SEL             0x030
#define VIRTIO_MMIO_QUEUE_NUM_MAX         0x034
#define VIRTIO_MMIO_QUEUE_NUM             0x038
#define VIRTIO_MMIO_QUEUE_READY           0x044
#define VIRTIO_MMIO_QUEUE_NOTIFY          0x050
#define VIRTIO_MMIO_INTERRUPT_STATUS      0x060
#define VIRTIO_MMIO_INTERRUPT_ACK         0x064
#define VIRTIO_MMIO_STATUS                0x070
#define VIRTIO_MMIO_QUEUE_DESC_LOW        0x080
#define VIRTIO_MMIO_QUEUE_DESC_HIGH       0x084
#define VIRTIO_MMIO_QUEUE_DRIVER_LOW      0x090
#define VIRTIO_MMIO_QUEUE_DRIVER_HIGH     0x094
#define VIRTIO_MMIO_QUEUE_DEVICE_LOW      0x0A0
#define VIRTIO_MMIO_QUEUE_DEVICE_HIGH     0x0A4

// Legacy (version 1) register offsets — virtio spec 1.0, Appendix D
#define VIRTIO_MMIO_LEGACY_HOST_FEATURES     0x010
#define VIRTIO_MMIO_LEGACY_GUEST_FEATURES    0x020
#define VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE   0x028
#define VIRTIO_MMIO_LEGACY_QUEUE_ALIGN       0x03C
#define VIRTIO_MMIO_LEGACY_QUEUE_PFN         0x040

// Magic value — virtio spec 1.1 §4.2.2.2
#define VIRTIO_MMIO_MAGIC  0x74726976U  // little-endian "virt"

// Device IDs — virtio spec 1.1, Appendix B
#define VIRTIO_DEVICE_ID_BLK    2
#define VIRTIO_DEVICE_ID_INPUT  18

// Device status bits — virtio spec 1.1 §2.1
#define VIRTIO_DEVICE_STATUS_ACKNOWLEDGE  (1U << 0)
#define VIRTIO_DEVICE_STATUS_DRIVER       (1U << 1)
#define VIRTIO_DEVICE_STATUS_DRIVER_OK    (1U << 2)
#define VIRTIO_DEVICE_STATUS_FEATURES_OK  (1U << 3)
#define VIRTIO_DEVICE_STATUS_NEEDS_RESET  (1U << 6)
#define VIRTIO_DEVICE_STATUS_FAILED       (1U << 7)

// Virtqueue descriptor flags — virtio spec 1.1 §2.7.5
#define VIRTQ_DESC_FLAG_NEXT   (1U << 0)
#define VIRTQ_DESC_FLAG_WRITE  (1U << 1)

// Interrupt status bits — virtio spec 1.1 §4.2.2.3
#define VIRTIO_MMIO_INT_VRING   (1U << 0)
#define VIRTIO_MMIO_INT_CONFIG  (1U << 1)

// virtio-blk request types — virtio spec 1.1 §5.2.6
#define VIRTIO_BLK_TYPE_IN   0  // read from device
#define VIRTIO_BLK_TYPE_OUT  1  // write to device

// virtio-blk status values — virtio spec 1.1 §5.2.6
#define VIRTIO_BLK_STATUS_OK          0
#define VIRTIO_BLK_STATUS_IOERR       1
#define VIRTIO_BLK_STATUS_UNSUPP      2
