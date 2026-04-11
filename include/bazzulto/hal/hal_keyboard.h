#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: Keyboard Input Device
//
// Platform-independent interface for keyboard hardware.
// The platform backend (e.g. virtio-input for QEMU virt) implements these.
// ---------------------------------------------------------------------------

// Initialize the keyboard device. If no keyboard is found, the driver
// remains dormant (hal_keyboard_get_irq_id returns 0).
void hal_keyboard_init(void);

// Handle the keyboard interrupt. Translates key events to characters
// and feeds them to the input layer via input_emit_char().
void hal_keyboard_irq_handler(void);

// Return the interrupt ID registered for this keyboard device.
// Returns 0 if no keyboard was found during init.
uint32_t hal_keyboard_get_irq_id(void);
