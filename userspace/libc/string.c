#include "string.h"

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
        while (count--)
            *destination_bytes++ = *source_bytes++;
    } else {
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

char *strtok_r(char *str, const char *delim, char **saveptr)
{
    if (str == (char *)0)
        str = *saveptr;

    str += strspn(str, delim);
    if (*str == '\0') {
        *saveptr = str;
        return (char *)0;
    }

    char *token_end = str + strcspn(str, delim);
    if (*token_end != '\0')
        *token_end++ = '\0';
    *saveptr = token_end;
    return str;
}

// WARNING: Uses static storage. Not safe for re-entrant or multi-threaded use.
// Future dynamic library migration: change static saveptr to __thread.
char *strtok(char *str, const char *delim)
{
    static char *saved_position = (char *)0;
    return strtok_r(str, delim, &saved_position);
}

char *strerror(int error_number)
{
    switch (error_number) {
        case 0:  return "Success";
        case 1:  return "Math argument out of domain";           // EDOM
        case 2:  return "Illegal byte sequence";                 // EILSEQ
        case 3:  return "Result out of range";                   // ERANGE
        case 4:  return "Operation not permitted";               // EPERM
        case 5:  return "No such file or directory";             // ENOENT
        case 6:  return "Bad file descriptor";                   // EBADF
        case 7:  return "Out of memory";                         // ENOMEM
        case 8:  return "Permission denied";                     // EACCES
        case 9:  return "File already exists";                   // EEXIST
        case 10: return "Not a directory";                       // ENOTDIR
        case 11: return "Is a directory";                        // EISDIR
        case 12: return "Invalid argument";                      // EINVAL
        case 13: return "Too many open file descriptors";        // EMFILE
        case 14: return "No space left on device";               // ENOSPC
        case 15: return "Illegal seek";                          // ESPIPE
        case 16: return "Function not implemented";              // ENOSYS
        case 17: return "Interrupted system call";               // EINTR
        case 18: return "Too many levels of symbolic links";     // ELOOP
        case 19: return "Read-only file system";                 // EROFS
        case 20: return "Directory not empty";                   // ENOTEMPTY
        case 21: return "Cross-device link";                     // EXDEV
        case 22: return "Bad address";                           // EFAULT
        case 23: return "No child processes";                    // ECHILD
        case 24: return "Resource temporarily unavailable";      // EAGAIN
        case 25: return "No such process";                       // ESRCH
        default: return "Unknown error";
    }
}
