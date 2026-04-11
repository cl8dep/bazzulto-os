#include "../../include/libc/stdio.h"
#include "../../include/libc/string.h"
#include <stdint.h>

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// Append one character to buf, respecting the size limit.
// pos is the current write position; size includes space for the null terminator.
static void append_char(char *buf, size_t *position, size_t size, char character)
{
    if (*position + 1 < size)
        buf[(*position)++] = character;
}

// Append a null-terminated string to buf.
static void append_string(char *buf, size_t *position, size_t size, const char *string)
{
    while (*string)
        append_char(buf, position, size, *string++);
}

// Append an unsigned 64-bit integer in the given base (10 or 16).
// uppercase controls whether hex digits use A-F or a-f.
static void append_unsigned(char *buf, size_t *position, size_t size,
                             uint64_t value, int base, int uppercase)
{
    const char *hex_digits = uppercase ? "0123456789ABCDEF" : "0123456789abcdef";
    char        reverse_buf[20];
    int         length = 0;

    if (value == 0) {
        append_char(buf, position, size, '0');
        return;
    }
    while (value) {
        reverse_buf[length++] = hex_digits[value % (uint64_t)base];
        value /= (uint64_t)base;
    }
    for (int index = length - 1; index >= 0; index--)
        append_char(buf, position, size, reverse_buf[index]);
}

// ---------------------------------------------------------------------------
// kvsnprintf — core formatter
// ---------------------------------------------------------------------------

int kvsnprintf(char *buf, size_t size, const char *format, va_list arguments)
{
    if (!buf || !size)
        return 0;

    size_t position = 0;

    while (*format) {
        if (*format != '%') {
            append_char(buf, &position, size, *format++);
            continue;
        }

        format++;   // skip '%'

        // Flags: '-' = left-justify.
        int left_justify = 0;
        if (*format == '-') {
            left_justify = 1;
            format++;
        }

        // Width: optional decimal integer.
        int width = 0;
        while (*format >= '0' && *format <= '9') {
            width = width * 10 + (*format - '0');
            format++;
        }

        // Length modifier: ll (long long), l (long — treated as 64-bit on AArch64).
        int is_long_long = 0;
        if (format[0] == 'l' && format[1] == 'l') {
            is_long_long = 1;
            format += 2;
        } else if (format[0] == 'l') {
            // On AArch64, long is 64-bit — promote to long-long treatment.
            is_long_long = 1;
            format++;
        }

        // Render the conversion into a small staging buffer, then apply width padding.
        char staging[64];
        size_t staging_position = 0;
        size_t staging_size = sizeof(staging);

#define STAGE_CHAR(c)   append_char(staging, &staging_position, staging_size, (c))
#define STAGE_STRING(s) append_string(staging, &staging_position, staging_size, (s))
#define STAGE_UINT(v,b,u) append_unsigned(staging, &staging_position, staging_size, (v), (b), (u))

        switch (*format) {
        case 's': {
            const char *s = va_arg(arguments, const char *);
            STAGE_STRING(s ? s : "(null)");
            break;
        }

        case 'c':
            STAGE_CHAR((char)va_arg(arguments, int));
            break;

        case 'd':
        case 'i': {
            int64_t value = is_long_long ? va_arg(arguments, int64_t)
                                         : (int64_t)va_arg(arguments, int);
            if (value < 0) {
                STAGE_CHAR('-');
                STAGE_UINT((uint64_t)-value, 10, 0);
            } else {
                STAGE_UINT((uint64_t)value, 10, 0);
            }
            break;
        }

        case 'u': {
            uint64_t value = is_long_long ? va_arg(arguments, uint64_t)
                                          : (uint64_t)va_arg(arguments, unsigned int);
            STAGE_UINT(value, 10, 0);
            break;
        }

        case 'x': {
            uint64_t value = is_long_long ? va_arg(arguments, uint64_t)
                                          : (uint64_t)va_arg(arguments, unsigned int);
            STAGE_UINT(value, 16, 0);
            break;
        }

        case 'X': {
            uint64_t value = is_long_long ? va_arg(arguments, uint64_t)
                                          : (uint64_t)va_arg(arguments, unsigned int);
            STAGE_UINT(value, 16, 1);
            break;
        }

        case 'p': {
            uint64_t value = (uint64_t)va_arg(arguments, void *);
            STAGE_STRING("0x");
            STAGE_UINT(value, 16, 0);
            break;
        }

        case '%':
            STAGE_CHAR('%');
            break;

        default:
            // Unknown specifier — emit literally so bugs are visible.
            STAGE_CHAR('%');
            if (is_long_long)
                STAGE_STRING("ll");
            STAGE_CHAR(*format);
            break;
        }

#undef STAGE_CHAR
#undef STAGE_STRING
#undef STAGE_UINT

        // Apply width padding: left-justify pads on the right, right-justify on the left.
        staging[staging_position] = '\0';
        int content_len = (int)staging_position;
        if (!left_justify) {
            for (int pad = content_len; pad < width; pad++)
                append_char(buf, &position, size, ' ');
        }
        append_string(buf, &position, size, staging);
        if (left_justify) {
            for (int pad = content_len; pad < width; pad++)
                append_char(buf, &position, size, ' ');
        }

        format++;
    }

    buf[position] = '\0';
    return (int)position;
}

// ---------------------------------------------------------------------------
// ksnprintf — public variadic entry point
// ---------------------------------------------------------------------------

int ksnprintf(char *buf, size_t size, const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    int result = kvsnprintf(buf, size, format, arguments);
    va_end(arguments);
    return result;
}
