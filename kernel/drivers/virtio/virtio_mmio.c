#include "../../../include/bazzulto/virtio_mmio.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/uart.h"

// Scan results — filled by virtio_mmio_enumerate().
static uint64_t enumerated_physical_bases[VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_slot_indices[VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_device_ids[VIRTIO_MMIO_NUM_SLOTS];
static int      enumerated_count = 0;

// Access a virtio-mmio register through the HHDM.
// physical_base is the base address of a single slot (e.g. 0x0A000000 + N*0x200).
static inline uint32_t virtio_read(uint64_t physical_base, uint32_t offset)
{
    return *(volatile uint32_t *)(hhdm_offset + physical_base + offset);
}

void virtio_mmio_enumerate(void)
{
    enumerated_count = 0;

    for (int slot = 0; slot < VIRTIO_MMIO_NUM_SLOTS; slot++) {
        uint64_t physical_base = VIRTIO_MMIO_BASE
                                 + (uint64_t)slot * VIRTIO_MMIO_SLOT_SIZE;

        // Confirm this slot contains a virtio device — virtio spec 1.1 §4.2.2.2.
        uint32_t magic     = virtio_read(physical_base, VIRTIO_MMIO_MAGIC_VALUE);
        uint32_t version   = virtio_read(physical_base, VIRTIO_MMIO_VERSION);
        uint32_t device_id = virtio_read(physical_base, VIRTIO_MMIO_DEVICE_ID);

        if (magic != VIRTIO_MMIO_MAGIC)
            continue;

        // Accept both version 1 (legacy) and version 2 (modern).
        // keyboard_init() selects the appropriate init sequence based on version.
        if (version != 1 && version != 2)
            continue;

        if (device_id == 0)
            continue;  // DeviceID 0 is reserved / no device present

        enumerated_physical_bases[enumerated_count] = physical_base;
        enumerated_slot_indices[enumerated_count]   = slot;
        enumerated_device_ids[enumerated_count]     = (int)device_id;
        enumerated_count++;

        uart_puts("[virtio] slot ");
        // Print slot number (single digit for now).
        char slot_char[3] = { '0' + (char)(slot / 10), '0' + (char)(slot % 10), '\0' };
        uart_puts(slot_char);
        uart_puts(" device_id=");
        // Print device_id as decimal.
        char id_buf[6];
        int id = (int)device_id;
        int id_len = 0;
        if (id == 0) { id_buf[id_len++] = '0'; }
        else { while (id > 0) { id_buf[id_len++] = '0' + (id % 10); id /= 10; } }
        // Reverse.
        for (int a = 0, b = id_len - 1; a < b; a++, b--) {
            char tmp = id_buf[a]; id_buf[a] = id_buf[b]; id_buf[b] = tmp;
        }
        id_buf[id_len] = '\0';
        uart_puts(id_buf);
        uart_puts("\n");
    }
}

uint64_t virtio_mmio_find_device(int device_id, int *slot_out)
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
