#include "stdio.h"
#include "string.h"
#include "unistd.h"
#include <stdint.h>

// ---------------------------------------------------------------------------
// Internal helpers (shared with kernel/lib/stdio.c logic)
// ---------------------------------------------------------------------------

static void append_char(char *buf, size_t *position, size_t size, char character)
{
    if (*position + 1 < size)
        buf[(*position)++] = character;
}

static void append_string(char *buf, size_t *position, size_t size, const char *string)
{
    while (*string)
        append_char(buf, position, size, *string++);
}

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
// vsnprintf — core formatter
// ---------------------------------------------------------------------------

int vsnprintf(char *buf, size_t size, const char *format, va_list arguments)
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

        // Flags.
        int left_justify = 0;
        if (*format == '-') { left_justify = 1; format++; }

        // Width.
        int width = 0;
        while (*format >= '0' && *format <= '9') { width = width * 10 + (*format - '0'); format++; }

        // Length modifier.
        int is_long_long = 0;
        if (format[0] == 'l' && format[1] == 'l') { is_long_long = 1; format += 2; }
        else if (format[0] == 'l') { is_long_long = 1; format++; }

        // Stage the conversion result before applying width padding.
        char staging[64];
        size_t staging_position = 0;
        size_t staging_size = sizeof(staging);
#define STAGE_CHAR(c)     append_char(staging, &staging_position, staging_size, (c))
#define STAGE_STRING(s)   append_string(staging, &staging_position, staging_size, (s))
#define STAGE_UINT(v,b,u) append_unsigned(staging, &staging_position, staging_size, (v), (b), (u))

        switch (*format) {
        case 's': {
            const char *s = va_arg(arguments, const char *);
            STAGE_STRING(s ? s : "(null)");
            break;
        }
        case 'c': STAGE_CHAR((char)va_arg(arguments, int)); break;
        case 'd':
        case 'i': {
            int64_t value = is_long_long ? va_arg(arguments, int64_t) : (int64_t)va_arg(arguments, int);
            if (value < 0) { STAGE_CHAR('-'); STAGE_UINT((uint64_t)-value, 10, 0); }
            else { STAGE_UINT((uint64_t)value, 10, 0); }
            break;
        }
        case 'u': STAGE_UINT(is_long_long ? va_arg(arguments, uint64_t) : (uint64_t)va_arg(arguments, unsigned int), 10, 0); break;
        case 'x': STAGE_UINT(is_long_long ? va_arg(arguments, uint64_t) : (uint64_t)va_arg(arguments, unsigned int), 16, 0); break;
        case 'X': STAGE_UINT(is_long_long ? va_arg(arguments, uint64_t) : (uint64_t)va_arg(arguments, unsigned int), 16, 1); break;
        case 'p': STAGE_STRING("0x"); STAGE_UINT((uint64_t)va_arg(arguments, void *), 16, 0); break;
        case '%': STAGE_CHAR('%'); break;
        default:
            STAGE_CHAR('%');
            if (is_long_long) STAGE_STRING("ll");
            STAGE_CHAR(*format);
            break;
        }

#undef STAGE_CHAR
#undef STAGE_STRING
#undef STAGE_UINT

        staging[staging_position] = '\0';
        int content_len = (int)staging_position;
        if (!left_justify) { for (int p = content_len; p < width; p++) append_char(buf, &position, size, ' '); }
        append_string(buf, &position, size, staging);
        if (left_justify)  { for (int p = content_len; p < width; p++) append_char(buf, &position, size, ' '); }

        format++;
    }

    buf[position] = '\0';
    return (int)position;
}

// ---------------------------------------------------------------------------
// snprintf
// ---------------------------------------------------------------------------

int snprintf(char *buf, size_t size, const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    int result = vsnprintf(buf, size, format, arguments);
    va_end(arguments);
    return result;
}

// ---------------------------------------------------------------------------
// printf — formats into a stack buffer then calls write(1, ...)
// ---------------------------------------------------------------------------

#define PRINTF_BUFFER_SIZE 512

int printf(const char *format, ...)
{
    char    buf[PRINTF_BUFFER_SIZE];
    va_list arguments;
    va_start(arguments, format);
    int length = vsnprintf(buf, sizeof(buf), format, arguments);
    va_end(arguments);
    if (length > 0)
        write(1, buf, (size_t)length);
    return length;
}

// ---------------------------------------------------------------------------
// sprintf — no size limit (uses an internal 65536-byte cap)
// ---------------------------------------------------------------------------

#define SPRINTF_BUFFER_SIZE 65536

int sprintf(char *buf, const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    int length = vsnprintf(buf, SPRINTF_BUFFER_SIZE, format, arguments);
    va_end(arguments);
    return length;
}

// ---------------------------------------------------------------------------
// fprintf — formats then writes to the given fd
// ---------------------------------------------------------------------------

int fprintf(int fd, const char *format, ...)
{
    char    buf[PRINTF_BUFFER_SIZE];
    va_list arguments;
    va_start(arguments, format);
    int length = vsnprintf(buf, sizeof(buf), format, arguments);
    va_end(arguments);
    if (length > 0)
        write(fd, buf, (size_t)length);
    return length;
}

// ---------------------------------------------------------------------------
// puts / putchar
// ---------------------------------------------------------------------------

int puts(const char *string)
{
    size_t length = strlen(string);
    int written = write(1, string, length);
    if (written < 0)
        return -1;
    write(1, "\n", 1);
    return written + 1;
}

int putchar(int character)
{
    unsigned char byte = (unsigned char)character;
    int result = write(1, (const char *)&byte, 1);
    return result < 0 ? -1 : character;
}

// ---------------------------------------------------------------------------
// getchar / fgets — input
// ---------------------------------------------------------------------------

int getchar(void)
{
    unsigned char byte;
    int64_t result = read(0, (char *)&byte, 1);
    return result <= 0 ? -1 : (int)byte;
}

char *fgets(char *buf, int size, int fd)
{
    if (size <= 0) return (char *)0;
    int i = 0;
    while (i < size - 1) {
        unsigned char byte;
        int64_t result = read(fd, (char *)&byte, 1);
        if (result <= 0) {
            if (i == 0) return (char *)0;  // EOF before any bytes
            break;
        }
        buf[i++] = (char)byte;
        if (byte == '\n') break;
    }
    buf[i] = '\0';
    return buf;
}
