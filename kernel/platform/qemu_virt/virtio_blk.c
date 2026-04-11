// virtio-blk block device driver for QEMU virt — virtio spec 1.1 §5.2
//
// Uses virtio-mmio legacy (v1) transport with a single requestq (queue 0).
// Each I/O operation is a 3-descriptor chain: header → data → status.
// I/O is synchronous: the calling code busy-waits via scheduler_yield()
// until the IRQ handler signals completion.
//
// IRQ routing: QEMU virt wires virtio-mmio slot N to GIC SPI (16+N),
// which is INTID (48+N). Source: QEMU hw/arm/virt.c.

#include "../../../include/bazzulto/hal/hal_disk.h"
#include "../../../include/bazzulto/hal/hal_irq.h"
#include "../../../include/bazzulto/hal/hal_virtio.h"
#include "../../../include/bazzulto/hal/hal_uart.h"
#include "../../../include/bazzulto/virtio_defs.h"
#include "../../../include/bazzulto/physical_memory.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/scheduler.h"
#include <string.h>

// QEMU virt: virtio-mmio slot N → SPI (16+N) → INTID (48+N).
#define IRQ_VIRTIO_MMIO_BASE  48

// ---------------------------------------------------------------------------
// virtqueue structures (virtio spec 1.1 §2.7) — same layout as keyboard
// ---------------------------------------------------------------------------

#define BLK_VIRTQUEUE_SIZE  16

typedef struct {
    uint64_t address;
    uint32_t length;
    uint16_t flags;
    uint16_t next;
} __attribute__((packed)) virtq_descriptor_t;

typedef struct {
    uint16_t flags;
    uint16_t idx;
    uint16_t ring[BLK_VIRTQUEUE_SIZE];
    uint16_t used_event;
} __attribute__((packed)) virtq_available_ring_t;

typedef struct {
    uint32_t id;
    uint32_t length;
} __attribute__((packed)) virtq_used_element_t;

typedef struct {
    uint16_t flags;
    uint16_t idx;
    virtq_used_element_t ring[BLK_VIRTQUEUE_SIZE];
    uint16_t avail_event;
} __attribute__((packed)) virtq_used_ring_t;

// ---------------------------------------------------------------------------
// virtio-blk request format — virtio spec 1.1 §5.2.6
// ---------------------------------------------------------------------------

typedef struct {
    uint32_t type;       // VIRTIO_BLK_TYPE_IN (read) or VIRTIO_BLK_TYPE_OUT (write)
    uint32_t reserved;
    uint64_t sector;     // starting 512-byte sector
} __attribute__((packed)) virtio_blk_request_header_t;

// ---------------------------------------------------------------------------
// Virtqueue memory layout (single 4KB page, legacy v1)
//
// QueueAlign = 256. With BLK_VIRTQUEUE_SIZE = 16:
//   [  0] descriptor_table  : 16 * 16 = 256 bytes
//   [256] available_ring    : 2+2+(2*16)+2 = 38 bytes → ends at 294
//   [294] alignment_pad     : 218 bytes to reach 512
//   [512] used_ring         : 2+2+(8*16)+2 = 134 bytes → ends at 646
//   [646] request_header    : 16 bytes
//   [662] status_byte       : 1 byte
//   [663] unused to end of page
// ---------------------------------------------------------------------------

#define BLK_VIRTQUEUE_ALIGN   256
#define BLK_PADDING_SIZE      218  // 512 - 294

typedef struct {
    virtq_descriptor_t     descriptor_table[BLK_VIRTQUEUE_SIZE];
    virtq_available_ring_t available_ring;
    uint8_t                alignment_pad[BLK_PADDING_SIZE];
    virtq_used_ring_t      used_ring;
    // Per-request data stored after the used ring:
    virtio_blk_request_header_t request_header;
    uint8_t                     status_byte;
} __attribute__((packed)) blk_virtqueue_t;

// ---------------------------------------------------------------------------
// Driver state
// ---------------------------------------------------------------------------

static blk_virtqueue_t *blk_virtqueue;
static uint64_t         blk_mmio_virtual_base;
static uint64_t         blk_queue_physical;       // physical address of the queue page
static uint16_t         blk_last_used_index;
static uint32_t         blk_irq_intid_value;
static uint64_t         blk_capacity_sectors;     // total 512-byte sectors

// Synchronous I/O signaling
static volatile int     blk_request_done;

// ---------------------------------------------------------------------------
// MMIO register helpers
// ---------------------------------------------------------------------------

static inline uint32_t blk_mmio_read(uint32_t offset)
{
    return *(volatile uint32_t *)(blk_mmio_virtual_base + offset);
}

static inline void blk_mmio_write(uint32_t offset, uint32_t value)
{
    *(volatile uint32_t *)(blk_mmio_virtual_base + offset) = value;
}

// ---------------------------------------------------------------------------
// Synchronous block I/O — submit a 3-descriptor chain and sleep until done
// ---------------------------------------------------------------------------

static int blk_do_request(uint32_t type, uint64_t sector,
                          void *data_buf, uint32_t data_len)
{
    if (!blk_virtqueue)
        return -1;

    // Physical address of the data buffer. The caller provides a kernel
    // virtual address (HHDM); convert to physical for the device.
    uint64_t data_physical = (uint64_t)data_buf - hhdm_offset;

    // Physical addresses of the header and status within the queue page.
    uint64_t header_physical = blk_queue_physical
        + __builtin_offsetof(blk_virtqueue_t, request_header);
    uint64_t status_physical = blk_queue_physical
        + __builtin_offsetof(blk_virtqueue_t, status_byte);

    // Fill the request header.
    blk_virtqueue->request_header.type     = type;
    blk_virtqueue->request_header.reserved = 0;
    blk_virtqueue->request_header.sector   = sector;
    blk_virtqueue->status_byte = 0xFF;  // sentinel — device will overwrite

    // Descriptor 0: request header (device-readable)
    blk_virtqueue->descriptor_table[0].address = header_physical;
    blk_virtqueue->descriptor_table[0].length  = sizeof(virtio_blk_request_header_t);
    blk_virtqueue->descriptor_table[0].flags   = VIRTQ_DESC_FLAG_NEXT;
    blk_virtqueue->descriptor_table[0].next    = 1;

    // Descriptor 1: data buffer
    blk_virtqueue->descriptor_table[1].address = data_physical;
    blk_virtqueue->descriptor_table[1].length  = data_len;
    blk_virtqueue->descriptor_table[1].flags   = VIRTQ_DESC_FLAG_NEXT
        | (type == VIRTIO_BLK_TYPE_IN ? VIRTQ_DESC_FLAG_WRITE : 0);
    blk_virtqueue->descriptor_table[1].next    = 2;

    // Descriptor 2: status byte (device-writable, always)
    blk_virtqueue->descriptor_table[2].address = status_physical;
    blk_virtqueue->descriptor_table[2].length  = 1;
    blk_virtqueue->descriptor_table[2].flags   = VIRTQ_DESC_FLAG_WRITE;
    blk_virtqueue->descriptor_table[2].next    = 0;

    // Place descriptor chain head (0) in the available ring.
    uint16_t avail_slot = blk_virtqueue->available_ring.idx % BLK_VIRTQUEUE_SIZE;
    blk_virtqueue->available_ring.ring[avail_slot] = 0;

    __asm__ volatile("dmb ish" ::: "memory");
    blk_virtqueue->available_ring.idx++;

    // Notify the device that there is a new request.
    __asm__ volatile("dmb ish" ::: "memory");
    blk_mmio_write(VIRTIO_MMIO_QUEUE_NOTIFY, 0);

    // Wait for device completion: enable IRQs so the handler can fire,
    // then WFI to yield the vCPU to QEMU's host event loop. When the device
    // completes, the IRQ fires, the handler updates blk_last_used_index,
    // and WFI returns. We then re-check the used ring.
    while (blk_last_used_index == blk_virtqueue->used_ring.idx) {
        __asm__ volatile("msr daifclr, #2");  // enable IRQs
        __asm__ volatile("wfi");               // yield vCPU to QEMU
        // WFI returns after IRQ handler ran (or spuriously)
        __asm__ volatile("msr daifset, #2");  // re-disable before re-check
    }
    blk_last_used_index = blk_virtqueue->used_ring.idx;

    __asm__ volatile("dmb ish" ::: "memory");

    // Check status byte.
    return (blk_virtqueue->status_byte == VIRTIO_BLK_STATUS_OK) ? 0 : -1;
}

// ---------------------------------------------------------------------------
// HAL implementation
// ---------------------------------------------------------------------------

int hal_disk_init(void)
{
    blk_virtqueue       = NULL;
    blk_mmio_virtual_base = 0;
    blk_last_used_index = 0;
    blk_irq_intid_value = 0;
    blk_capacity_sectors = 0;

    // Find the virtio-blk device (DeviceID 2).
    int slot = 0;
    uint64_t physical_base = hal_virtio_find_device(VIRTIO_DEVICE_ID_BLK, &slot);
    if (physical_base == 0) {
        hal_uart_puts("[blk] no virtio-blk device found\n");
        return -1;
    }

    blk_mmio_virtual_base = hhdm_offset + physical_base;

    // --- virtio legacy (v1) initialization — virtio spec 1.0 §4.2.3 ---

    // Step 1: Reset
    blk_mmio_write(VIRTIO_MMIO_STATUS, 0);

    // Step 2: ACKNOWLEDGE
    blk_mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_DEVICE_STATUS_ACKNOWLEDGE);

    // Step 3: DRIVER
    blk_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE | VIRTIO_DEVICE_STATUS_DRIVER);

    // Step 4: Feature negotiation — accept no features for simplicity.
    (void)blk_mmio_read(VIRTIO_MMIO_LEGACY_HOST_FEATURES);
    blk_mmio_write(VIRTIO_MMIO_LEGACY_GUEST_FEATURES, 0);

    // Step 5: Guest page size
    blk_mmio_write(VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE, PAGE_SIZE);

    // Read device capacity — virtio spec 1.1 §5.2.4
    // Legacy: config space starts at MMIO offset 0x100.
    // capacity is a uint64_t at offset 0 of the config space.
    uint32_t cap_lo = blk_mmio_read(0x100);
    uint32_t cap_hi = blk_mmio_read(0x104);
    blk_capacity_sectors = ((uint64_t)cap_hi << 32) | (uint64_t)cap_lo;

    // Step 6: Set up virtqueue 0 (requestq)
    blk_queue_physical = (uint64_t)physical_memory_alloc();
    if (blk_queue_physical == 0) {
        hal_uart_puts("[blk] failed to allocate virtqueue page\n");
        blk_mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_DEVICE_STATUS_FAILED);
        return -1;
    }

    blk_virtqueue = (blk_virtqueue_t *)(hhdm_offset + blk_queue_physical);

    // Zero the entire page.
    uint8_t *page_bytes = (uint8_t *)blk_virtqueue;
    for (int i = 0; i < PAGE_SIZE; i++)
        page_bytes[i] = 0;

    blk_mmio_write(VIRTIO_MMIO_QUEUE_SEL, 0);

    uint32_t queue_num_max = blk_mmio_read(VIRTIO_MMIO_QUEUE_NUM_MAX);
    if (queue_num_max < BLK_VIRTQUEUE_SIZE) {
        hal_uart_puts("[blk] device queue too small\n");
        blk_mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_DEVICE_STATUS_FAILED);
        return -1;
    }

    blk_mmio_write(VIRTIO_MMIO_QUEUE_NUM, BLK_VIRTQUEUE_SIZE);
    blk_mmio_write(VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, BLK_VIRTQUEUE_ALIGN);
    blk_mmio_write(VIRTIO_MMIO_LEGACY_QUEUE_PFN,
        (uint32_t)(blk_queue_physical / PAGE_SIZE));

    // Step 7: Enable GIC interrupt for this slot.
    blk_irq_intid_value = (uint32_t)(IRQ_VIRTIO_MMIO_BASE + slot);
    hal_irq_enable(blk_irq_intid_value);

    // Step 8: DRIVER_OK
    blk_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE |
        VIRTIO_DEVICE_STATUS_DRIVER      |
        VIRTIO_DEVICE_STATUS_DRIVER_OK);

    hal_uart_puts("[blk] virtio-blk initialized, capacity=");
    // Print capacity in sectors as decimal.
    uint64_t cap_print = blk_capacity_sectors;
    char cap_buf[24];
    int cap_len = 0;
    if (cap_print == 0) { cap_buf[cap_len++] = '0'; }
    else { while (cap_print > 0) { cap_buf[cap_len++] = '0' + (int)(cap_print % 10); cap_print /= 10; } }
    for (int a = 0, b = cap_len - 1; a < b; a++, b--) {
        char tmp = cap_buf[a]; cap_buf[a] = cap_buf[b]; cap_buf[b] = tmp;
    }
    cap_buf[cap_len] = '\0';
    hal_uart_puts(cap_buf);
    hal_uart_puts(" sectors\n");

    return 0;
}

uint32_t hal_disk_get_irq_id(void)
{
    return blk_irq_intid_value;
}

void hal_disk_irq_handler(void)
{
    if (!blk_virtqueue)
        return;

    // Acknowledge the interrupt.
    uint32_t interrupt_status = blk_mmio_read(VIRTIO_MMIO_INTERRUPT_STATUS);
    blk_mmio_write(VIRTIO_MMIO_INTERRUPT_ACK, interrupt_status);

    if (!(interrupt_status & VIRTIO_MMIO_INT_VRING))
        return;

    __asm__ volatile("dmb ish" ::: "memory");

    // Drain the used ring — for our synchronous single-request model,
    // there should be exactly one entry.
    while (blk_last_used_index != blk_virtqueue->used_ring.idx) {
        blk_last_used_index++;
    }

    // Signal the waiting code that the request is complete.
    blk_request_done = 1;
}

int hal_disk_read_sectors(uint64_t lba, uint32_t count, void *buffer)
{
    if (!blk_virtqueue || count == 0)
        return -1;

    // Read one chunk at a time. The data buffer must be in kernel virtual
    // (HHDM) space so we can compute the physical address for the device.
    // For multi-sector reads, we issue one request per sector to keep the
    // buffer management simple (no need for physically contiguous multi-page
    // allocations). QEMU handles this efficiently.
    uint8_t *dst = (uint8_t *)buffer;
    for (uint32_t i = 0; i < count; i++) {
        if (blk_do_request(VIRTIO_BLK_TYPE_IN, lba + i, dst + i * 512, 512) < 0)
            return -1;
    }
    return 0;
}

int hal_disk_write_sectors(uint64_t lba, uint32_t count, const void *buffer)
{
    if (!blk_virtqueue || count == 0)
        return -1;

    const uint8_t *src = (const uint8_t *)buffer;
    for (uint32_t i = 0; i < count; i++) {
        // Cast away const — the device reads from this buffer, it won't modify it.
        if (blk_do_request(VIRTIO_BLK_TYPE_OUT, lba + i, (void *)(src + i * 512), 512) < 0)
            return -1;
    }
    return 0;
}

uint64_t hal_disk_capacity(void)
{
    return blk_capacity_sectors;
}
