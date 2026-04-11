#pragma once

#include <stdint.h>
#include "../../limine/limine.h"

// Initialize the console using the framebuffer provided by Limine.
// Must be called before any console_print* functions.
void console_init(struct limine_framebuffer *framebuffer);

// Print a null-terminated string to the console.
void console_print(const char *str);

// Print a null-terminated string followed by a newline.
void console_println(const char *str);

// Clear the console — fill the entire framebuffer with the background color
// and reset the cursor to (0, 0).
void console_clear(void);

// Write a single character to the console.
// '\n' advances to the next line.
// '\r' moves the cursor to the start of the current line (carriage return only).
// '\r' followed by '\n' produces the standard terminal new-line behavior.
void console_putc(char c);
