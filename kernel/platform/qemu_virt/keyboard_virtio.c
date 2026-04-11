#include "../../../include/bazzulto/hal/hal_keyboard.h"
#include "../../../include/bazzulto/keymap.h"
#include "../../lib/utf8.h"
#include "../../../include/bazzulto/virtio_defs.h"
#include "../../../include/bazzulto/hal/hal_virtio.h"
#include "../../../include/bazzulto/input.h"
#include "../../../include/bazzulto/tty.h"
#include "../../../include/bazzulto/hal/hal_irq.h"
#include "../../../include/bazzulto/physical_memory.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/hal/hal_uart.h"
#include <stddef.h>

// ---------------------------------------------------------------------------
// virtio-input keyboard driver
//
// Protocol: virtio spec 1.1, §5.8 (Input Device) and §2.7 (Virtqueues).
// The keyboard presents one virtqueue (eventq, index 0). We pre-populate it
// with KEYBOARD_VIRTQUEUE_SIZE write-only descriptors. When the user presses
// a key, the device fills one event buffer and moves the descriptor to the
// used ring. The IRQ handler drains the used ring, translates the event to
// ASCII, and feeds it to the input layer via tty_receive_char().
//
// IRQ routing: QEMU virt wires virtio-mmio slot N to GIC SPI (16+N),
// which is INTID (48+N). Source: QEMU hw/arm/virt.c.
// ---------------------------------------------------------------------------

#define KEYBOARD_VIRTQUEUE_SIZE  16   // number of descriptors in eventq

// EV_KEY event type — matches Linux evdev / virtio-input wire format
#define VIRTIO_INPUT_EV_KEY  1U

// QEMU virt: virtio-mmio slot N → SPI (16+N) → INTID (48+N).
// Source: QEMU hw/arm/virt.c, VIRT_MMIO_IRQ_BASE = 16, SPIs start at INTID 32.
#define IRQ_VIRTIO_MMIO_BASE  48

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

// ----- Keymap-based translation -----
//
// The active keymap is populated at init time from an embedded .bkm file.
// When VFS is available, it will be loaded from //system:/etc/keymaps/.

static keymap_t active_keymap;

// Dead key state: if non-zero, the next keypress should be composed.
static uint8_t pending_dead_key;

// Fallback US QWERTY keymap embedded as a string (used until VFS can load .bkm files).
// This is a minimal subset — the full keymap lives in resources/keymaps/us.bkm.
static const char embedded_us_keymap[] =
    "1 \\e \\e -\n"
    "2 1 ! -\n"  "3 2 @ -\n"  "4 3 # -\n"  "5 4 $ -\n"
    "6 5 % -\n"  "7 6 ^ -\n"  "8 7 & {\n"  "9 8 * [\n"
    "10 9 ( ]\n" "11 0 ) }\n" "12 - _ \\\n" "13 = + -\n"
    "14 \\b \\b -\n" "15 \\t \\t -\n"
    "16 q Q -\n" "17 w W -\n" "18 e E -\n" "19 r R -\n"
    "20 t T -\n" "21 y Y -\n" "22 u U -\n" "23 i I -\n"
    "24 o O -\n" "25 p P -\n" "26 [ { -\n" "27 ] } ~\n"
    "28 \\n \\n -\n"
    "30 a A -\n" "31 s S -\n" "32 d D -\n" "33 f F -\n"
    "34 g G -\n" "35 h H -\n" "36 j J -\n" "37 k K -\n"
    "38 l L -\n" "39 ; : -\n" "40 ' \" -\n" "41 ` ~ -\n"
    "43 \\\\ | -\n"
    "44 z Z -\n" "45 x X -\n" "46 c C -\n" "47 v V -\n"
    "48 b B -\n" "49 n N -\n" "50 m M -\n"
    "51 , < -\n" "52 . > -\n" "53 / ? -\n"
    "57 \\s \\s -\n";

// Linux evdev keycode constants for modifier keys
#define KEYCODE_LEFT_SHIFT   42
#define KEYCODE_RIGHT_SHIFT  54
#define KEYCODE_CAPS_LOCK    58
#define KEYCODE_LEFT_CTRL    29
#define KEYCODE_RIGHT_CTRL   97
#define KEYCODE_LEFT_ALT     56
#define KEYCODE_RIGHT_ALT    100  // AltGr on international keyboards

// ----- Driver state -----

static keyboard_virtqueue_t *keyboard_virtqueue;  // virtqueue memory (one physical page)
static uint64_t keyboard_mmio_virtual_base;       // hhdm-mapped register base
static uint16_t keyboard_last_used_index;         // tracks our position in used ring
static uint32_t keyboard_irq_intid_value;         // GIC INTID for this device's IRQ

// Modifier state
static int keyboard_shift_held;    // non-zero if either shift key is down
static int keyboard_ctrl_held;     // non-zero if either ctrl key is down
static int keyboard_altgr_held;    // non-zero if AltGr (Right Alt) is down
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

// ----- Key event translation (keymap-based) -----

// Emit a UTF-8 string (possibly multi-byte) one byte at a time through the
// input layer. tty_receive_char() is called for each byte.
static void keyboard_emit_utf8(const char *str)
{
    while (*str) {
        tty_receive_char(*str);
        str++;
    }
}

static void keyboard_handle_key_event(uint16_t keycode, uint32_t event_value)
{
    // Update modifier state on both key-down and key-up.
    if (keycode == KEYCODE_LEFT_SHIFT || keycode == KEYCODE_RIGHT_SHIFT) {
        keyboard_shift_held = (event_value != INPUT_EVENT_VALUE_KEY_UP);
        return;
    }
    if (keycode == KEYCODE_LEFT_CTRL || keycode == KEYCODE_RIGHT_CTRL) {
        keyboard_ctrl_held = (event_value != INPUT_EVENT_VALUE_KEY_UP);
        return;
    }
    if (keycode == KEYCODE_RIGHT_ALT) {
        keyboard_altgr_held = (event_value != INPUT_EVENT_VALUE_KEY_UP);
        return;
    }
    if (keycode == KEYCODE_LEFT_ALT) {
        return;  // Left Alt — not used for character input yet
    }
    if (keycode == KEYCODE_CAPS_LOCK) {
        if (event_value == INPUT_EVENT_VALUE_KEY_DOWN)
            keyboard_capslock_active = !keyboard_capslock_active;
        return;
    }

    // Only emit a character on key-down or autorepeat — not on key-up.
    if (event_value == INPUT_EVENT_VALUE_KEY_UP)
        return;

    if (keycode >= KEYMAP_MAX_EVDEV_CODE)
        return;

    // Determine modifier for keymap lookup.
    // Shift+AltGr (4th level) takes priority, then AltGr, then Shift.
    int modifier = KEYMAP_MODIFIER_NORMAL;
    if (keyboard_shift_held && keyboard_altgr_held)
        modifier = KEYMAP_MODIFIER_SHIFT_ALTGR;
    else if (keyboard_altgr_held)
        modifier = KEYMAP_MODIFIER_ALTGR;
    else if (keyboard_shift_held)
        modifier = KEYMAP_MODIFIER_SHIFT;

    const char *mapping = keymap_lookup(&active_keymap, keycode, modifier);

    // If Shift+AltGr has no mapping, fall back to AltGr, then Shift, then normal.
    if (mapping[0] == '\0' && modifier == KEYMAP_MODIFIER_SHIFT_ALTGR)
        mapping = keymap_lookup(&active_keymap, keycode, KEYMAP_MODIFIER_ALTGR);
    if (mapping[0] == '\0' && modifier >= KEYMAP_MODIFIER_ALTGR)
        mapping = keymap_lookup(&active_keymap, keycode, KEYMAP_MODIFIER_SHIFT);
    if (mapping[0] == '\0')
        mapping = keymap_lookup(&active_keymap, keycode, KEYMAP_MODIFIER_NORMAL);
    if (mapping[0] == '\0')
        return;

    // Check for dead key.
    uint8_t dead = keymap_is_dead_key(mapping);
    if (dead) {
        if (pending_dead_key) {
            // Dead key chaining: emit the pending dead key literally, keep the new one.
            char literal[KEYMAP_MAX_CHAR_BYTES];
            int literal_len = keymap_dead_key_literal(pending_dead_key, literal);
            if (literal_len > 0)
                keyboard_emit_utf8(literal);
        }
        pending_dead_key = dead;
        return;
    }

    // If a dead key is pending, try to compose.
    if (pending_dead_key) {
        char composed[KEYMAP_MAX_CHAR_BYTES];
        int composed_len = keymap_compose_dead_key(pending_dead_key, mapping, composed);
        uint8_t saved_dead = pending_dead_key;
        pending_dead_key = 0;
        if (composed_len > 0) {
            composed[composed_len] = '\0';
            keyboard_emit_utf8(composed);
            return;
        }
        // No compose rule — emit the dead key literal, then the base character.
        char literal[KEYMAP_MAX_CHAR_BYTES];
        int literal_len = keymap_dead_key_literal(saved_dead, literal);
        if (literal_len > 0)
            keyboard_emit_utf8(literal);
        // Fall through to emit the base character below.
    }

    // Apply caps lock for alphabetic characters (ASCII and Latin-1).
    // For ASCII: flip a↔A. For Latin-1 accented: flip á↔Á, ñ↔Ñ, etc.
    char adjusted[KEYMAP_MAX_CHAR_BYTES];
    if (keyboard_capslock_active) {
        const char *tmp = mapping;
        uint32_t codepoint = utf8_decode(&tmp);
        uint32_t flipped = 0;

        // ASCII letters
        if (codepoint >= 'a' && codepoint <= 'z')
            flipped = codepoint - 'a' + 'A';
        else if (codepoint >= 'A' && codepoint <= 'Z')
            flipped = codepoint - 'A' + 'a';
        // Latin-1 lowercase (U+00E0..U+00F6, U+00F8..U+00FE) ↔ uppercase (U+00C0..U+00DE)
        else if (codepoint >= 0xC0 && codepoint <= 0xD6)
            flipped = codepoint + 0x20;
        else if (codepoint >= 0xD8 && codepoint <= 0xDE)
            flipped = codepoint + 0x20;
        else if (codepoint >= 0xE0 && codepoint <= 0xF6)
            flipped = codepoint - 0x20;
        else if (codepoint >= 0xF8 && codepoint <= 0xFE)
            flipped = codepoint - 0x20;

        if (flipped) {
            int len = utf8_encode(flipped, adjusted);
            adjusted[len] = '\0';
            mapping = adjusted;
        }
    }

    // Ctrl+letter → control codes (0x01–0x1A).
    if (keyboard_ctrl_held && mapping[1] == '\0') {
        char character = mapping[0];
        if (character >= 'a' && character <= 'z')
            character = (char)(character - 'a' + 1);
        else if (character >= 'A' && character <= 'Z')
            character = (char)(character - 'A' + 1);
        tty_receive_char(character);
        return;
    }

    keyboard_emit_utf8(mapping);
}

// ----- IRQ handler -----

void hal_keyboard_irq_handler(void)
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

uint32_t hal_keyboard_get_irq_id(void)
{
    return keyboard_irq_intid_value;
}

// ----- Initialization -----

void hal_keyboard_init(void)
{
    keyboard_virtqueue        = NULL;
    keyboard_mmio_virtual_base = 0;
    keyboard_last_used_index   = 0;
    keyboard_irq_intid_value   = 0;
    keyboard_shift_held        = 0;
    keyboard_ctrl_held         = 0;
    keyboard_altgr_held        = 0;
    keyboard_capslock_active   = 0;
    pending_dead_key           = 0;

    // Load the embedded US QWERTY keymap.
    // When VFS is available, this will be replaced by loading from
    // //system:/etc/keymaps/ based on keyboard.conf.
    keymap_parse(embedded_us_keymap, sizeof(embedded_us_keymap) - 1, &active_keymap);

    // Find the virtio-input device (DeviceID 18).
    int slot = 0;
    uint64_t physical_base = hal_virtio_find_device(VIRTIO_DEVICE_ID_INPUT, &slot);
    if (physical_base == 0) {
        hal_uart_puts("[keyboard] no virtio-input device found — keyboard disabled\n");
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
        hal_uart_puts("[keyboard] failed to allocate virtqueue page — aborting\n");
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
        hal_uart_puts("[keyboard] device queue too small — aborting\n");
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
    hal_irq_enable(keyboard_irq_intid_value);

    // Step 8: Set DRIVER_OK — the device may now start sending events.
    keyboard_mmio_write(VIRTIO_MMIO_STATUS,
        VIRTIO_DEVICE_STATUS_ACKNOWLEDGE |
        VIRTIO_DEVICE_STATUS_DRIVER      |
        VIRTIO_DEVICE_STATUS_DRIVER_OK);

    hal_uart_puts("[keyboard] virtio-input initialized, INTID=");
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
    hal_uart_puts(intid_buf);
    hal_uart_puts("\n");
}
