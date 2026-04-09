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
