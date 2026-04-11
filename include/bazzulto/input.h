#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// Input abstraction layer
//
// All character input — whether from a virtio keyboard, a UART serial port,
// or any future source (USB HID, etc.) — is funneled through this layer.
// Drivers call input_emit_char() from their IRQ handlers.
// Consumers (VFS stdin) call input_getchar(), which blocks until a character
// is available.
//
// The layer also accepts raw input events for future consumers (e.g. a GUI)
// that need keycodes rather than ASCII characters.
// ---------------------------------------------------------------------------

// Raw input event — matches the Linux evdev format and the virtio-input
// wire format (virtio spec §5.8.6.1).
typedef struct {
    uint16_t type;   // EV_KEY = 1, EV_SYN = 0, etc.
    uint16_t code;   // Linux evdev keycode (KEY_A = 30, KEY_SPACE = 57, ...)
    int32_t  value;  // 0 = key up, 1 = key down, 2 = autorepeat
} input_event_t;

// Event type constants (evdev EV_* values)
#define INPUT_EVENT_TYPE_SYN  0   // EV_SYN — synchronisation marker
#define INPUT_EVENT_TYPE_KEY  1   // EV_KEY — keyboard key event

// Event value constants for EV_KEY
#define INPUT_EVENT_VALUE_KEY_UP      0
#define INPUT_EVENT_VALUE_KEY_DOWN    1
#define INPUT_EVENT_VALUE_KEY_REPEAT  2

// Initialize the input layer. Must be called before any driver calls
// input_emit_char() or before input_getchar() is used.
void input_init(void);

// Enqueue a translated ASCII character into the input ring buffer.
// Safe to call from IRQ context.
// Characters are silently dropped if the ring buffer is full.
void input_emit_char(char character);

// Block the calling process until an ASCII character is available, then
// return it. Called by the VFS console read path for stdin (fd 0).
// Returns -1 if interrupted by a pending signal (so the syscall can return
// and deliver the signal before eret).
int input_getchar(void);
