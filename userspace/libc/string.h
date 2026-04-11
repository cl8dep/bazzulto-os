#pragma once

#include <stddef.h>

// Memory operations — C11 §7.24.1
void  *memset(void *destination, int value, size_t count);
void  *memcpy(void *destination, const void *source, size_t count);
void  *memmove(void *destination, const void *source, size_t count);
int    memcmp(const void *a, const void *b, size_t count);
void  *memchr(const void *s, int c, size_t n);

// String operations — C11 §7.24.2 / §7.24.3 / §7.24.4 / §7.24.5
size_t strlen(const char *string);
int    strcmp(const char *a, const char *b);
int    strncmp(const char *a, const char *b, size_t count);
char  *strcpy(char *destination, const char *source);
char  *strncpy(char *destination, const char *source, size_t count);
char  *strchr(const char *string, int character);
char  *strrchr(const char *string, int character);
char  *strcat(char *destination, const char *source);
char  *strncat(char *destination, const char *source, size_t count);
char  *strstr(const char *haystack, const char *needle);
size_t strspn(const char *s, const char *accept);
size_t strcspn(const char *s, const char *reject);
char  *strpbrk(const char *s, const char *accept);

// Re-entrant tokenizer — POSIX.1-2008.
char  *strtok_r(char *str, const char *delim, char **saveptr);

// Non-re-entrant tokenizer — C11 §7.24.5.8.
// WARNING: Uses static storage. Not safe in multi-threaded or recursive contexts.
// Future dynamic library: change static saveptr to __thread for thread safety.
char  *strtok(char *str, const char *delim);
