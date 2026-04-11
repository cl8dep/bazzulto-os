#pragma once

#include <stdint.h>

// Display the boot splash screen on the framebuffer.
// shell_pid         — PID of the shell process (shown in the info table).
// keyboard_irq_intid — GIC INTID of the keyboard IRQ, or 0 if no keyboard found.
void splash_display(uint16_t shell_pid, uint32_t keyboard_irq_intid);
