#pragma once

#include <stddef.h>
#include <stdarg.h>

// Kernel-only formatted string functions.
// These do NOT call printf or any libc function — safe to use before full init.

// Write a formatted string into buf (at most size bytes, always null-terminated).
// Supported conversions: %s %c %d %i %u %x %X %lld %llu %llx %llX %p %%
// Returns the number of characters written (not counting the null terminator).
int ksnprintf(char *buf, size_t size, const char *format, ...)
    __attribute__((format(printf, 3, 4)));

// va_list variant — useful when wrapping ksnprintf inside another variadic function.
int kvsnprintf(char *buf, size_t size, const char *format, va_list arguments);
