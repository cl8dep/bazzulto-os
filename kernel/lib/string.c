#include "../../include/libc/string.h"

// ---------------------------------------------------------------------------
// Memory operations
// ---------------------------------------------------------------------------

void *memset(void *destination, int value, size_t count)
{
    unsigned char *pointer = (unsigned char *)destination;
    unsigned char fill    = (unsigned char)value;
    while (count--)
        *pointer++ = fill;
    return destination;
}

void *memcpy(void *destination, const void *source, size_t count)
{
    unsigned char       *destination_bytes = (unsigned char *)destination;
    const unsigned char *source_bytes      = (const unsigned char *)source;
    while (count--)
        *destination_bytes++ = *source_bytes++;
    return destination;
}

void *memmove(void *destination, const void *source, size_t count)
{
    unsigned char       *destination_bytes = (unsigned char *)destination;
    const unsigned char *source_bytes      = (const unsigned char *)source;

    if (destination_bytes < source_bytes || destination_bytes >= source_bytes + count) {
        // Non-overlapping or destination is before source — forward copy is safe.
        while (count--)
            *destination_bytes++ = *source_bytes++;
    } else {
        // Destination overlaps source from behind — copy backwards to avoid clobber.
        destination_bytes += count;
        source_bytes      += count;
        while (count--)
            *--destination_bytes = *--source_bytes;
    }
    return destination;
}

int memcmp(const void *a, const void *b, size_t count)
{
    const unsigned char *pointer_a = (const unsigned char *)a;
    const unsigned char *pointer_b = (const unsigned char *)b;
    while (count--) {
        if (*pointer_a != *pointer_b)
            return (int)*pointer_a - (int)*pointer_b;
        pointer_a++;
        pointer_b++;
    }
    return 0;
}

// ---------------------------------------------------------------------------
// String operations
// ---------------------------------------------------------------------------

size_t strlen(const char *string)
{
    size_t length = 0;
    while (string[length])
        length++;
    return length;
}

int strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t count)
{
    while (count && *a && *a == *b) {
        a++;
        b++;
        count--;
    }
    if (!count)
        return 0;
    return (unsigned char)*a - (unsigned char)*b;
}

char *strcpy(char *destination, const char *source)
{
    char *start = destination;
    while ((*destination++ = *source++))
        ;
    return start;
}

char *strncpy(char *destination, const char *source, size_t count)
{
    char *start = destination;
    while (count && (*destination++ = *source++))
        count--;
    while (count--)
        *destination++ = '\0';
    return start;
}

char *strchr(const char *string, int character)
{
    while (*string) {
        if (*string == (char)character)
            return (char *)string;
        string++;
    }
    return (character == '\0') ? (char *)string : (char *)0;
}

char *strrchr(const char *string, int character)
{
    const char *last_occurrence = (char *)0;
    while (*string) {
        if (*string == (char)character)
            last_occurrence = string;
        string++;
    }
    if (character == '\0')
        return (char *)string;
    return (char *)last_occurrence;
}

char *strcat(char *destination, const char *source)
{
    char *end = destination + strlen(destination);
    while ((*end++ = *source++))
        ;
    return destination;
}

char *strncat(char *destination, const char *source, size_t count)
{
    char *end = destination + strlen(destination);
    while (count && *source) {
        *end++ = *source++;
        count--;
    }
    *end = '\0';
    return destination;
}

// ---------------------------------------------------------------------------
// C11 §7.24.5 — Additional memory search
// ---------------------------------------------------------------------------

void *memchr(const void *s, int c, size_t n)
{
    const unsigned char *pointer = (const unsigned char *)s;
    unsigned char target = (unsigned char)c;
    while (n--) {
        if (*pointer == target)
            return (void *)pointer;
        pointer++;
    }
    return (void *)0;
}

// ---------------------------------------------------------------------------
// C11 §7.24.5 — String search
// ---------------------------------------------------------------------------

char *strstr(const char *haystack, const char *needle)
{
    if (!*needle)
        return (char *)haystack;
    size_t needle_length = strlen(needle);
    while (*haystack) {
        if (*haystack == *needle &&
            strncmp(haystack, needle, needle_length) == 0)
            return (char *)haystack;
        haystack++;
    }
    return (char *)0;
}

size_t strspn(const char *s, const char *accept)
{
    size_t count = 0;
    while (*s) {
        const char *a = accept;
        int found = 0;
        while (*a) {
            if (*s == *a++) { found = 1; break; }
        }
        if (!found) break;
        s++;
        count++;
    }
    return count;
}

size_t strcspn(const char *s, const char *reject)
{
    size_t count = 0;
    while (*s) {
        const char *r = reject;
        while (*r) {
            if (*s == *r++) return count;
        }
        s++;
        count++;
    }
    return count;
}

char *strpbrk(const char *s, const char *accept)
{
    while (*s) {
        const char *a = accept;
        while (*a) {
            if (*s == *a) return (char *)s;
            a++;
        }
        s++;
    }
    return (char *)0;
}

// ---------------------------------------------------------------------------
// POSIX.1-2008 — Re-entrant tokenizer (no global state; safe in kernel)
// ---------------------------------------------------------------------------

char *strtok_r(char *str, const char *delim, char **saveptr)
{
    if (str == (char *)0)
        str = *saveptr;

    // Skip leading delimiters.
    str += strspn(str, delim);
    if (*str == '\0') {
        *saveptr = str;
        return (char *)0;
    }

    // Find the end of the current token.
    char *token_end = str + strcspn(str, delim);
    if (*token_end != '\0')
        *token_end++ = '\0';
    *saveptr = token_end;
    return str;
}
