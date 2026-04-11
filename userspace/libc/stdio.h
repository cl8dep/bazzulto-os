#pragma once

#include <stddef.h>
#include <stdarg.h>

// Write a formatted string into buf (at most size bytes, always null-terminated).
// Supported conversions: %s %c %d %i %u %x %X %lld %llu %llx %llX %p %%
// Returns the number of characters written (not counting the null terminator).
int snprintf(char *buf, size_t size, const char *format, ...)
    __attribute__((format(printf, 3, 4)));

// va_list variant.
int vsnprintf(char *buf, size_t size, const char *format, va_list arguments);

// Print a formatted string to stdout (fd 1).
// Returns the number of characters written.
int printf(const char *format, ...)
    __attribute__((format(printf, 1, 2)));

// Format into buf with no size limit (uses an internal 65536-byte cap).
// WARNING: Caller must ensure buf is large enough — no overflow protection.
int sprintf(char *buf, const char *format, ...)
    __attribute__((format(printf, 2, 3)));

// Format then write to the given file descriptor.
// Returns the number of characters written.
int fprintf(int fd, const char *format, ...)
    __attribute__((format(printf, 2, 3)));

// Write string followed by newline to stdout (fd 1).
// Returns number of bytes written, or -1 on error.
int puts(const char *string);

// Write a single character to stdout (fd 1).
// Returns the character written, or -1 on error.
int putchar(int character);

// Read a single character from stdin (fd 0).
// Returns the character, or -1 on EOF/error.
int getchar(void);

// Read at most size-1 characters from stdin into buf, stopping at '\n' or EOF.
// Always null-terminates. Returns buf on success, NULL on EOF with no bytes read.
char *fgets(char *buf, int size, int fd);
