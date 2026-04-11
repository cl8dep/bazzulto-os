#pragma once

#include <stddef.h>
#include <stdint.h>

// String-to-integer conversions — C11 §7.22.1
// Note: errno is not set on overflow (no errno in freestanding kernel).
long               strtol(const char *str, char **endptr, int base);
unsigned long      strtoul(const char *str, char **endptr, int base);
long long          strtoll(const char *str, char **endptr, int base);
unsigned long long strtoull(const char *str, char **endptr, int base);

// Convenience wrappers — C11 §7.22.1.2 / §7.22.1.3 / §7.22.1.4
int       atoi(const char *str);
long      atol(const char *str);
long long atoll(const char *str);

// Integer arithmetic — C11 §7.22.6
int       abs(int n);
long      labs(long n);
long long llabs(long long n);

// Process control — C11 §7.22.4
// exit() is declared in userspace/library/systemcall.h (calls SYS_EXIT).
void abort(void) __attribute__((noreturn));

// Pseudo-random number generation — C11 §7.22.2
#define RAND_MAX 2147483647
int  rand(void);
void srand(unsigned int seed);

// Searching and sorting — C11 §7.22.5
void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compare)(const void *, const void *));
void  qsort(void *base, size_t nmemb, size_t size,
            int (*compare)(const void *, const void *));

// Dynamic memory allocation — C11 §7.22.3
// Backed by the mmap syscall. NULL is returned on allocation failure.
// Returned pointers are 32-byte aligned (satisfies max_align_t on AArch64).
void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t new_size);
void  free(void *ptr);
