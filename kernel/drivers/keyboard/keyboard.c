#include "../../../include/bazzulto/keyboard.h"
#include "../../../include/bazzulto/virtio_mmio.h"
#include "../../../include/bazzulto/input.h"
#include "../../../include/bazzulto/gic.h"
#include "../../../include/bazzulto/physical_memory.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/uart.h"
#include <stddef.h>

// ---------------------------------------------------------------------------
// virtio-input keyboard driver
//
// Protocol: virtio spec 1.1, §5.8 (Input Device) and §2.7 (Virtqueues).
// The keyboard presents one virtqueue (eventq, index 0). We pre-populate it
// with KEYBOARD_VIRTQUEUE_SIZE write-only descriptors. When the user presses
// a key, the device fills one event buffer and moves the descriptor to the
// used ring. The IRQ handler drains the used ring, translates the event to
// ASCII, and feeds it to the input layer via input_emit_char().
//
// IRQ routing: QEMU virt wires virtio-mmio slot N to GIC SPI (16+N),
// which is INTID (48+N). Source: QEMU hw/arm/virt.c.
// ---------------------------------------------------------------------------

#define KEYBOARD_VIRTQUEUE_SIZE  16   // number of descriptors in eventq

// EV_KEY event type — matches Linux evdev / virtio-input wire format
#define VIRTIO_INPUT_EV_KEY  1U

// ----- virtqueue structures (virtio spec 1.1 §2.7) -----

// §2.7.5 — Descriptor Table entry
typedef struct {
    uint64_t address;   // physical address of the buffer
    uint32_t length;    // byte length of the buffer
    uint16_t flags;     // VIRTQ_DESC_FLAG_* bits
    uint16_t next;      // index of next descriptor (if VIRTQ_DESC_FLAG_NEXT)
} __attribute__((packed)) virtq_descriptor_t;

// §2.7.6 — Available Ring (driver → device)
typedef struct {
    uint16_t flags;                              // 0 = normal operation
    uint16_t idx;                                // next free slot in ring[]
    uint16_t ring[KEYBOARD_VIRTQUEUE_SIZE];      // descriptor head indices
    uint16_t used_event;                         // suppress IRQ notification (unused)
} __attribute__((packed)) virtq_available_ring_t;

// §2.7.8 — Used Ring element
typedef struct {
    uint32_t id;      // descriptor head index returned by device
    uint32_t length;  // bytes written by device into the buffer
} __attribute__((packed)) virtq_used_element_t;

// §2.7.8 — Used Ring (device → driver)
typedef struct {
    uint16_t flags;                                   // 0 = normal operation
    uint16_t idx;                                     // next free slot in ring[]
    virtq_used_element_t ring[KEYBOARD_VIRTQUEUE_SIZE];
    uint16_t avail_event;                             // suppress notification (unused)
} __attribute__((packed)) virtq_used_ring_t;

// §5.8.6.1 — virtio-input event wire format
typedef struct {
    uint16_t type;   // EV_KEY = 1
    uint16_t code;   // Linux evdev keycode
    uint32_t value;  // 0 = up, 1 = down, 2 = repeat
} __attribute__((packed)) virtio_input_event_t;

// Legacy (version 1) virtio-mmio requires all three rings to be in one
// contiguous block starting at QueuePFN * PAGE_SIZE. The used ring must start
// at the first QueueAlign-aligned offset past the end of the available ring.
//
// We use QueueAlign = 256. With KEYBOARD_VIRTQUEUE_SIZE = 16:
//   [  0] descriptor_table  : 16 * 16 = 256 bytes
//   [256] available_ring    : 2+2+(2*16)+2 = 38 bytes   → ends at 294
//   [294] alignment_pad     : 218 bytes to reach 512 (first 256-aligned ≥ 294)
//   [512] used_ring         : 2+2+(8*16)+2 = 134 bytes  → ends at 646
//   [646] event_buffers     : 16 * 8 = 128 bytes         → total 774 bytes
// All fits in one 4096-byte physical page.
#define KEYBOARD_VIRTQUEUE_ALIGN  256
#define KEYBOARD_USED_RING_OFFSET 512  // first ALIGN-multiple ≥ (256+38=294)
#define KEYBOARD_PADDING_SIZE     218  // 512 - 294

typedef struct {
    virtq_descriptor_t     descriptor_table[KEYBOARD_VIRTQUEUE_SIZE];  // offset   0
    virtq_available_ring_t available_ring;                               // offset 256
    uint8_t                alignment_pad[KEYBOARD_PADDING_SIZE];         // offset 294
    virtq_used_ring_t      used_ring;                                    // offset 512
    virtio_input_event_t   event_buffers[KEYBOARD_VIRTQUEUE_SIZE];       // offset 646
} __attribute__((packed)) keyboard_virtqueue_t;

// ----- US QWERTY keycode → ASCII translation tables -----
//
// Indexed by Linux evdev keycode (0-127).
// 0 = no ASCII mapping (modifier key, function key, etc.).
// Source: linux/input-event-codes.h

static const char keycode_to_ascii_normal[128] = {
    //  0          1(ESC)    2          3          4          5          6          7
        0,         0x1B,     '1',       '2',       '3',       '4',       '5',       '6',
    //  8          9          10         11         12         13         14(BS)     15(TAB)
        '7',       '8',       '9',       '0',       '-',       '=',       0x7F,      '\t',
    //  16(Q)      17(W)      18(E)      19(R)      20(T)      21(Y)      22(U)      23(I)
        'q',       'w',       'e',       'r',       't',       'y',       'u',       'i',
    //  24(O)      25(P)      26([)      27(])      28(ENTER)  29(LCTRL)  30(A)      31(S)
        'o',       'p',       '[',       ']',       '\r',      0,         'a',       's',
    //  32(D)      33(F)      34(G)      35(H)      36(J)      37(K)      38(L)      39(;)
        'd',       'f',       'g',       'h',       'j',       'k',       'l',       ';',
    //  40(')      41(`)      42(LSHIFT) 43(\)      44(Z)      45(X)      46(C)      47(V)
        '\'',      '`',       0,         '\\',      'z',       'x',       'c',       'v',
    //  48(B)      49(N)      50(M)      51(,)      52(.)      53(/)      54(RSHIFT) 55(KP*)
        'b',       'n',       'm',       ',',       '.',       '/',       0,         '*',
    //  56(LALT)   57(SPACE)  58(CAPS)   59-63 (F1-F5)
        0,         ' ',       0,         0,         0,         0,         0,         0,
    // 64-127: F6+, numpad, arrows, etc. — no ASCII mapping for now
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
};

static const char keycode_to_ascii_shift[128] = {
    //  0          1(ESC)     2(!)       3(@)       4(#)       5($)       6(%)       7(^)
        0,         0x1B,      '!',       '@',       '#',       '$',       '%',       '^',
    //  8(&)       9(*)       10(()      11())      12(_)      13(+)      14(BS)     15(TAB)
        '&',       '*',       '(',       ')',       '_',       '+',       0x7F,      '\t',
    //  16(Q)      17(W)      18(E)      19(R)      20(T)      21(Y)      22(U)      23(I)
        'Q',       'W',       'E',       'R',       'T',       'Y',       'U',       'I',
    //  24(O)      25(P)      26({)      27(})      28(ENTER)  29(LCTRL)  30(A)      31(S)
        'O',       'P',       '{',       '}',       '\r',      0,         'A',       'S',
    //  32(D)      33(F)      34(G)      35(H)      36(J)      37(K)      38(L)      39(:)
        'D',       'F',       'G',       'H',       'J',       'K',       'L',       ':',
    //  40(")      41(~)      42(LSHIFT) 43(|)      44(Z)      45(X)      46(C)      47(V)
        '"',       '~',       0,         '|',       'Z',       'X',       'C',       'V',
    //  48(B)      49(N)      50(M)      51(<)      52(>)      53(?)      54(RSHIFT) 55(KP*)
        'B',       'N',       'M',       '<',       '>',       '?',       0,         '*',
    //  56(LALT)   57(SPACE)  58(CAPS)   59-63
        0,         ' ',       0,         0,         0,         0,         0,         0,
    // 64-127
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
};

// Linux evdev keycode constants for modifier keys
#define KEYCODE_LEFT_SHIFT   42
#define KEYCODE_RIGHT_SHIFT  54
#define KEYCODE_CAPS_LOCK    58

// ----- Driver state -----

static keyboard_virtqueue_t *keyboard_virtqueue;  // virtqueue memory (one physical page)
static uint64_t keyboard_mmio_virtual_base;       // hhdm-mapped register base
static uint16_t keyboard_last_used_index;         // tracks our position in used ring
static uint32_t keyboard_irq_intid_value;         // GIC INTID for this device's IRQ

// Modifier state
static int keyboard_shift_held;    // non-zero if either shift key is down
static int keyboard_capslock_active;

// ----- MMIO register helpers -----

static inline uint32_t keyboard_mmio_read(uint32_t offset)
{
    return *(volatile uint32_t *)(keyboard_mmio_virtual_base + offset);
}

static inline void keyboard_mmio_write(uint32_t offset, uint32_t value)
{
    *(volatile uint32_t *)(keyboard_mmio_virtual_base + offset) = value;
}

// ----- Key event translation -----

static void keyboard_handle_key_event(uint16_t keycode, uint32_t event_value)
{
    // Update modifier state on both key-down and key-up.
    if (keycode == KEYCODE_LEFT_SHIFT || keycode == KEYCODE_RIGHT_SHIFT) {
        keyboard_shift_held = (event_value != INPUT_EVENT_VALUE_KEY_UP);
        return;
    }
    if (keycode == KEYCODE_CAPS_LOCK) {
        if (event_value == INPUT_EVENT_VALUE_KEY_DOWN)
            keyboard_capslock_active = !keyboard_capslock_active;
        return;
    }

    // Only emit a character on key-down or autorepeat — not on key-up.
    if (event_value == INPUT_EVENT_VALUE_KEY_UP)
        return;

    if (keycode >= 128)
        return;

    // Select character from the appropriate table.
    char character;
    if (keyboard_shift_held) {
        character = keycode_to_ascii_shift[keycode];
    } else {
        character = keycode_to_ascii_normal[keycode];
    }

    // Apply caps lock: flip the case of alphabetic characters only.
    if (keyboard_capslock_active && character >= 'a' && character <= 'z')
        character = (char)(character - 'a' + 'A');
    else if (keyboard_capslock_active && character >= 'A' && character <= 'Z')
        character = (char)(character - 'A' + 'a');

    if (character != 0)
        input_emit_char(character);
}

// ----- IRQ handler -----

void keyboard_irq_handler(void)
{
    if (!keyboard_virtqueue)
        return;

    // Acknowledge the interrupt — virtio spec 1.1 §4.2.2.3.
    uint32_t interrupt_status = keyboard_mmio_read(VIRTIO_MMIO_INTERRUPT_STATUS);
    keyboard_mmio_write(VIRTIO_MMIO_INTERRUPT_ACK, interrupt_status);

    if (!(interrupt_status & VIRTIO_MMIO_INT_VRING))
        return;  // not a used-ring update (e.g. config change) — nothing to drain

    // AArch64 data memory barrier — ensure we read used_ring.idx after the
    // IRQ acknowledge, not speculatively before it.
    // DDI 0487 §B2.3 — dmb ish covers inner-shareable domain.
    __asm__ volatile("dmb ish" ::: "memory");

    // Drain all entries the device has placed in the used ring since we last
    // checked. keyboard_last_used_index tracks our read position.
    while (keyboard_last_used_index != keyboard_virtqueue->used_ring.idx) {
        uint32_t position = keyboard_last_used_index % KEYBOARD_VIRTQUEUE_SIZE;
        uint32_t descriptor_index = keyboard_virtqueue->used_ring.ring[position].id;

        virtio_input_event_t *event =
            &keyboard_virtqueue->event_buffers[descriptor_index];

        if (event->type == VIRTIO_INPUT_EV_KEY)
            keyboard_handle_key_event(event->code, event->value);

        // Return the descriptor to the available ring so the device can reuse it.
        uint16_t available_slot =
            keyboard_virtqueue->available_ring.idx % KEYBOARD_VIRTQUEUE_SIZE;
        keyboard_virtqueue->available_ring.ring[available_slot] =
            (uint16_t)descriptor_index;

        // Barrier before incrementing available_ring.idx — the device must see
        // the ring[] write before the idx update (virtio spec 1.1 §2.7.13).
        __asm__ volatile("dmb ish" ::: "memory");
        keyboard_virtqueue->available_ring.idx++;

        keyboard_last_used_index++;
    }

    // Notify the device that the available ring has new entries.
    // Queue index 0 = eventq — virtio spec 1.1 §5.8.
    __asm__ volatile("dmb ish" ::: "memory");
    keyboard_mmio_write(VIRTIO_MMIO_QUEUE_NOTIFY, 0);
}

uint32_t keyboard_get_irq_intid(void)
{
    return keyboard_irq_intid_value;
}

// ----- Initialization -----

void keyboard_init(void)
{
    keyboard_virtqueue        = NULL;
    keyboard_mmio_virtual_base = 0;
    keyboard_last_used_index   = 0;
    keyboard_irq_intid_value   = 0;
    keyboard_shift_held        = 0;
    keyboard_capslock_active   = 0;

    // Find the virtio-input device (DeviceID 18).
    int slot = 0;
    uint64_t physical_base = virtio_mmio_find_device(VIRTIO_DEVICE_ID_INPUT, &slot);
    if (physical_base == 0) {
        uart_puts("[keyboard] no virtio-input device found — keyboard disabled\n");
        return;
    }

    keyboard_mmio_virtual_base = hhdm_offset + physical_base;

    // ----- virtio legacy (version 1) initialization — virtio spec 1.0 §4.2.3 -----
    // Version 2 (non-legacy) is not used: QEMU virt returns version=1 for
    // virtio-mmio devices on this machine configuration.

    // Step 1: Reset the device by writing 0 to STATUS.
    keyboard_mmio_write(VIRTIO_MMIO_STATUS, 0);

    // Step 2: Set ACKNOWLEDGE — we have seen the device.
    keyboard_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE);

    // Step 3: Set DRIVER — we know how to drive this device.
    keyboard_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE |
        VIRTIO_DEVICE_STATUS_DRIVER);

    // Step 4: Feature negotiation (legacy uses single HostFeatures/GuestFeatures).
    // virtio-input requires no special features; accept zero.
    (void)keyboard_mmio_read(VIRTIO_MMIO_LEGACY_HOST_FEATURES);
    keyboard_mmio_write(VIRTIO_MMIO_LEGACY_GUEST_FEATURES, 0);

    // Step 5: Tell the device our page size so it can interpret QueuePFN.
    // virtio spec 1.0 §4.2.3.2 — GuestPageSize must be set before QueuePFN.
    keyboard_mmio_write(VIRTIO_MMIO_LEGACY_GUEST_PAGE_SIZE, PAGE_SIZE);

    // Step 6: Set up virtqueue 0 (eventq).

    // Allocate one physical page for the entire virtqueue state.
    // All three rings must be in this single contiguous block (legacy requirement).
    uint64_t queue_physical = (uint64_t)physical_memory_alloc();
    if (queue_physical == 0) {
        uart_puts("[keyboard] failed to allocate virtqueue page — aborting\n");
        keyboard_mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_DEVICE_STATUS_FAILED);
        return;
    }

    keyboard_virtqueue =
        (keyboard_virtqueue_t *)(hhdm_offset + queue_physical);

    // Zero the entire page so all ring indices start at 0.
    uint8_t *page_bytes = (uint8_t *)keyboard_virtqueue;
    for (int byte_index = 0; byte_index < PAGE_SIZE; byte_index++)
        page_bytes[byte_index] = 0;

    // Select queue 0 and configure its size.
    keyboard_mmio_write(VIRTIO_MMIO_QUEUE_SEL, 0);

    uint32_t queue_num_max = keyboard_mmio_read(VIRTIO_MMIO_QUEUE_NUM_MAX);
    if (queue_num_max < KEYBOARD_VIRTQUEUE_SIZE) {
        uart_puts("[keyboard] device queue too small — aborting\n");
        keyboard_mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_DEVICE_STATUS_FAILED);
        return;
    }

    keyboard_mmio_write(VIRTIO_MMIO_QUEUE_NUM, KEYBOARD_VIRTQUEUE_SIZE);

    // Set used-ring alignment within the queue block.
    // virtio spec 1.0 §4.2.3.2: the used ring must start at a QueueAlign-byte
    // aligned offset from the descriptor table base. We use 256 so the used
    // ring starts at offset 512, as laid out in keyboard_virtqueue_t.
    keyboard_mmio_write(VIRTIO_MMIO_LEGACY_QUEUE_ALIGN, KEYBOARD_VIRTQUEUE_ALIGN);

    // Give the device the physical page frame number of the queue block.
    // The device derives all ring addresses from this single base pointer.
    keyboard_mmio_write(VIRTIO_MMIO_LEGACY_QUEUE_PFN,
        (uint32_t)(queue_physical / PAGE_SIZE));

    // Pre-populate all KEYBOARD_VIRTQUEUE_SIZE descriptors. Each descriptor
    // points to one event buffer and is marked WRITE (device fills the buffer).
    // All are placed in the available ring so the device can use them immediately.
    for (int descriptor_index = 0;
         descriptor_index < KEYBOARD_VIRTQUEUE_SIZE;
         descriptor_index++) {
        uint64_t event_physical =
            queue_physical
            + __builtin_offsetof(keyboard_virtqueue_t, event_buffers)
            + (uint64_t)descriptor_index * sizeof(virtio_input_event_t);

        keyboard_virtqueue->descriptor_table[descriptor_index].address =
            event_physical;
        keyboard_virtqueue->descriptor_table[descriptor_index].length  =
            sizeof(virtio_input_event_t);
        keyboard_virtqueue->descriptor_table[descriptor_index].flags   =
            VIRTQ_DESC_FLAG_WRITE;
        keyboard_virtqueue->descriptor_table[descriptor_index].next    = 0;

        keyboard_virtqueue->available_ring.ring[descriptor_index] =
            (uint16_t)descriptor_index;
    }

    // Memory barrier: descriptor writes must be visible before the idx update.
    __asm__ volatile("dmb ish" ::: "memory");
    keyboard_virtqueue->available_ring.idx = KEYBOARD_VIRTQUEUE_SIZE;

    // Step 7: Register and enable the GIC interrupt for this slot.
    // QEMU virt: virtio-mmio slot N → SPI (16+N) → INTID (48+N).
    // Source: QEMU hw/arm/virt.c, VIRT_MMIO_IRQ_BASE = 16, SPIs start at INTID 32.
    keyboard_irq_intid_value = (uint32_t)(IRQ_VIRTIO_MMIO_BASE + slot);
    gic_enable_spi(keyboard_irq_intid_value);

    // Step 8: Set DRIVER_OK — the device may now start sending events.
    keyboard_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE |
        VIRTIO_DEVICE_STATUS_DRIVER      |
        VIRTIO_DEVICE_STATUS_DRIVER_OK);

    uart_puts("[keyboard] virtio-input initialized, INTID=");
    // Print INTID as decimal.
    uint32_t intid_print = keyboard_irq_intid_value;
    char intid_buf[6];
    int intid_len = 0;
    if (intid_print == 0) { intid_buf[intid_len++] = '0'; }
    else { while (intid_print > 0) { intid_buf[intid_len++] = '0' + (int)(intid_print % 10); intid_print /= 10; } }
    for (int a = 0, b = intid_len - 1; a < b; a++, b--) {
        char tmp = intid_buf[a]; intid_buf[a] = intid_buf[b]; intid_buf[b] = tmp;
    }
    intid_buf[intid_len] = '\0';
    uart_puts(intid_buf);
    uart_puts("\n");
}
