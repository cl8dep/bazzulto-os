// Virtio-MMIO bus enumeration for QEMU virt
//
// QEMU virt: 32 virtio-mmio slots at physical 0x0A000000, each 0x200 bytes.
// IRQ for slot N = GIC SPI (16+N) = INTID (48+N).
// Source: QEMU hw/arm/virt.c

#include "../../../include/bazzulto/hal/hal_virtio.h"
#include "../../../include/bazzulto/hal/hal_uart.h"
#include "../../../include/bazzulto/virtio_defs.h"
#include "../../../include/bazzulto/kernel.h"

// QEMU virt platform-specific constants
#define QEMU_VIRTIO_MMIO_BASE      0x0A000000ULL
#define QEMU_VIRTIO_MMIO_SLOT_SIZE 0x200
#define QEMU_VIRTIO_MMIO_NUM_SLOTS 32

// Scan results
static uint64_t enumerated_physical_bases[QEMU_VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_slot_indices[QEMU_VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_device_ids[QEMU_VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_count = 0;

static inline uint32_t virtio_read(uint64_t physical_base, uint32_t offset)
{
    return *(volatile uint32_t *)(hhdm_offset + physical_base + offset);
}

void hal_virtio_enumerate(void)
{
    enumerated_count = 0;

    for (int slot = 0; slot < QEMU_VIRTIO_MMIO_NUM_SLOTS; slot++) {
        uint64_t physical_base = QEMU_VIRTIO_MMIO_BASE
                                 + (uint64_t)slot * QEMU_VIRTIO_MMIO_SLOT_SIZE;

        uint32_t magic     = virtio_read(physical_base, VIRTIO_MMIO_MAGIC_VALUE);
        uint32_t version   = virtio_read(physical_base, VIRTIO_MMIO_VERSION);
        uint32_t device_id = virtio_read(physical_base, VIRTIO_MMIO_DEVICE_ID);

        if (magic != VIRTIO_MMIO_MAGIC)
            continue;
        if (version != 1 && version != 2)
            continue;
        if (device_id == 0)
            continue;

        enumerated_physical_bases[enumerated_count] = physical_base;
        enumerated_slot_indices[enumerated_count]   = slot;
        enumerated_device_ids[enumerated_count]     = (int)device_id;
        enumerated_count++;

        hal_uart_puts("[virtio] slot ");
        char slot_char[3] = { '0' + (char)(slot / 10), '0' + (char)(slot % 10), '\0' };
        hal_uart_puts(slot_char);
        hal_uart_puts(" device_id=");
        char id_buf[6];
        int id = (int)device_id;
        int id_len = 0;
        if (id == 0) { id_buf[id_len++] = '0'; }
        else { while (id > 0) { id_buf[id_len++] = '0' + (id % 10); id /= 10; } }
        for (int a = 0, b = id_len - 1; a < b; a++, b--) {
            char tmp = id_buf[a]; id_buf[a] = id_buf[b]; id_buf[b] = tmp;
        }
        id_buf[id_len] = '\0';
        hal_uart_puts(id_buf);
        hal_uart_puts("\n");
    }
}

uint64_t hal_virtio_find_device(int device_id, int *slot_out)
{
    for (int i = 0; i < enumerated_count; i++) {
        if (enumerated_device_ids[i] == device_id) {
            if (slot_out)
                *slot_out = enumerated_slot_indices[i];
            return enumerated_physical_bases[i];
        }
    }
    return 0;
}
