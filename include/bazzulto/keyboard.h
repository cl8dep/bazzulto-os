#pragma once

#include <stdint.h>

// Initialize the virtio-input keyboard driver.
// Scans virtio-mmio for a device with DeviceID 18 (input), initializes the
// virtqueue, registers the GIC IRQ, and enables the device.
// Safe to call even if no keyboard device is present — logs a warning and
// returns without error.
// Must be called after: virtual memory active, heap ready, input_init().
void keyboard_init(void);

// Called from the IRQ dispatcher when the keyboard's virtio-mmio SPI fires.
// Drains the used ring, translates EV_KEY events to ASCII, and calls
// input_emit_char() for each printable character.
void keyboard_irq_handler(void);

// Return the GIC INTID registered for the keyboard device, or 0 if
// keyboard_init() found no device.
// Used by the IRQ dispatcher to route the correct interrupt.
uint32_t keyboard_get_irq_intid(void);
