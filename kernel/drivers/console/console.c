#include "../../../include/bazzulto/console.h"
#include "font_latin_extended.h"
#include "../../lib/utf8.h"
#include <stddef.h>

// Console state
static uint32_t *framebuffer_address;
static uint32_t framebuffer_width;
static uint32_t framebuffer_pitch_in_pixels;
static uint32_t cursor_x;  // in character columns
static uint32_t cursor_y;  // in character rows
static uint32_t columns;
static uint32_t rows;

#define CHAR_WIDTH  8
#define CHAR_HEIGHT 8
#define COLOR_FG    0x00F8F8F2  // off-white
#define COLOR_BG    0x00272822  // dark gray

// Draw a single Unicode codepoint at the given character cell.
static void draw_codepoint(uint32_t codepoint, uint32_t col, uint32_t row) {
    const uint8_t *glyph = font_latin_extended_lookup(codepoint);
    uint32_t pixel_x = col * CHAR_WIDTH;
    uint32_t pixel_y = row * CHAR_HEIGHT;

    for (int y = 0; y < CHAR_HEIGHT; y++) {
        for (int x = 0; x < CHAR_WIDTH; x++) {
            uint32_t color = (glyph[y] & (1 << x)) ? COLOR_FG : COLOR_BG;
            framebuffer_address[(pixel_y + y) * framebuffer_pitch_in_pixels + (pixel_x + x)] = color;
        }
    }
}


static void scroll_up(void) {
    // Move every row up by one character height by copying pixel rows
    for (uint32_t y = 0; y < (rows - 1) * CHAR_HEIGHT; y++) {
        uint32_t *dst = &framebuffer_address[y * framebuffer_pitch_in_pixels];
        uint32_t *src = &framebuffer_address[(y + CHAR_HEIGHT) * framebuffer_pitch_in_pixels];
        for (uint32_t x = 0; x < framebuffer_width; x++) {
            dst[x] = src[x];
        }
    }
    // Clear the last row
    for (uint32_t y = (rows - 1) * CHAR_HEIGHT; y < rows * CHAR_HEIGHT; y++) {
        uint32_t *row_ptr = &framebuffer_address[y * framebuffer_pitch_in_pixels];
        for (uint32_t x = 0; x < framebuffer_width; x++) {
            row_ptr[x] = COLOR_BG;
        }
    }
}

void console_init(struct limine_framebuffer *fb) {
    // pitch is in bytes; divide by 4 to get 32-bit pixel units
    framebuffer_address     = (uint32_t *)fb->address;
    framebuffer_width       = fb->width;
    framebuffer_pitch_in_pixels = fb->pitch / 4;
    columns = fb->width  / CHAR_WIDTH;
    rows    = fb->height / CHAR_HEIGHT;
    cursor_x = 0;
    cursor_y = 0;

    // Fill background
    for (uint32_t y = 0; y < fb->height; y++) {
        for (uint32_t x = 0; x < fb->width; x++) {
            framebuffer_address[y * framebuffer_pitch_in_pixels + x] = COLOR_BG;
        }
    }
}

void console_clear(void) {
    uint32_t height = rows * CHAR_HEIGHT;
    for (uint32_t y = 0; y < height; y++) {
        for (uint32_t x = 0; x < framebuffer_width; x++)
            framebuffer_address[y * framebuffer_pitch_in_pixels + x] = COLOR_BG;
    }
    cursor_x = 0;
    cursor_y = 0;
}

void console_print(const char *str) {
    while (*str) {
        if (*str == '\n') {
            cursor_x = 0;
            cursor_y++;
            str++;
        } else {
            // Decode one UTF-8 codepoint and advance the pointer.
            uint32_t codepoint = utf8_decode(&str);
            draw_codepoint(codepoint, cursor_x, cursor_y);
            cursor_x++;
            if (cursor_x >= columns) {
                cursor_x = 0;
                cursor_y++;
            }
        }

        if (cursor_y >= rows) {
            scroll_up();
            cursor_y = rows - 1;
        }
    }
}

void console_println(const char *str) {
    console_print(str);
    console_print("\n");
}

// UTF-8 accumulation buffer for console_putc().
// When bytes arrive one at a time (from the input layer), we accumulate
// continuation bytes until a full codepoint is assembled.
static char     utf8_buffer[4];
static int      utf8_buffer_index;
static int      utf8_expected_length;

void console_putc(char c) {
    unsigned char byte = (unsigned char)c;

    if (c == '\r') {
        cursor_x = 0;
        return;
    }
    if (c == '\b' || c == 0x7F) {
        if (cursor_x > 0)
            cursor_x--;
        return;
    }
    if (c == '\n') {
        cursor_x = 0;
        cursor_y++;
        if (cursor_y >= rows) { scroll_up(); cursor_y = rows - 1; }
        return;
    }

    // UTF-8 multi-byte accumulation.
    // If this is a lead byte (not a continuation), start a new sequence.
    if ((byte & 0x80) == 0) {
        // ASCII — emit immediately.
        utf8_buffer_index = 0;
        utf8_expected_length = 0;
        draw_codepoint((uint32_t)byte, cursor_x, cursor_y);
        cursor_x++;
    } else if ((byte & 0xC0) != 0x80) {
        // Lead byte of a multi-byte sequence.
        utf8_buffer[0] = c;
        utf8_buffer_index = 1;
        utf8_expected_length = utf8_codepoint_size((const char *)&byte);
        if (utf8_expected_length <= 1) {
            // Invalid lead — emit replacement.
            draw_codepoint(0xFFFD, cursor_x, cursor_y);
            cursor_x++;
            utf8_buffer_index = 0;
            utf8_expected_length = 0;
        }
        // Wait for continuation bytes.
        return;
    } else {
        // Continuation byte.
        if (utf8_buffer_index == 0 || utf8_buffer_index >= 4) {
            // Unexpected continuation — emit replacement.
            draw_codepoint(0xFFFD, cursor_x, cursor_y);
            cursor_x++;
            utf8_buffer_index = 0;
            utf8_expected_length = 0;
        } else {
            utf8_buffer[utf8_buffer_index++] = c;
            if (utf8_buffer_index < utf8_expected_length)
                return;  // still waiting for more bytes

            // Complete sequence — decode and draw.
            const char *ptr = utf8_buffer;
            uint32_t codepoint = utf8_decode(&ptr);
            draw_codepoint(codepoint, cursor_x, cursor_y);
            cursor_x++;
            utf8_buffer_index = 0;
            utf8_expected_length = 0;
        }
    }

    if (cursor_x >= columns) {
        cursor_x = 0;
        cursor_y++;
    }
    if (cursor_y >= rows) {
        scroll_up();
        cursor_y = rows - 1;
    }
}
