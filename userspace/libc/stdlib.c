#include "stdlib.h"

// exit() is provided by userspace/library/systemcall.h; forward-declare here
// so abort() can call it without pulling in the full syscall header.
extern void exit(int status) __attribute__((noreturn));

// ---------------------------------------------------------------------------
// Core conversion — strtoull
// Handles: leading whitespace, optional sign, base-0 auto-detect
// (0x/0X → hex, leading 0 → octal, else decimal), overflow clamping.
// No errno — document in header.
// ---------------------------------------------------------------------------

unsigned long long strtoull(const char *str, char **endptr, int base)
{
    const char *cursor = str;

    // Skip leading whitespace.
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\n' ||
           *cursor == '\r' || *cursor == '\f' || *cursor == '\v')
        cursor++;

    // Optional sign (unsigned result — '-' wraps, '+' ignored).
    int negative = 0;
    if (*cursor == '-') { negative = 1; cursor++; }
    else if (*cursor == '+') { cursor++; }

    // Base auto-detection.
    if (base == 0) {
        if (cursor[0] == '0' && (cursor[1] == 'x' || cursor[1] == 'X')) {
            base = 16;
        } else if (cursor[0] == '0') {
            base = 8;
        } else {
            base = 10;
        }
    }

    // Skip '0x' / '0X' prefix for base 16.
    if (base == 16 && cursor[0] == '0' &&
        (cursor[1] == 'x' || cursor[1] == 'X'))
        cursor += 2;

    // Accumulate digits.
    unsigned long long result   = 0;
    unsigned long long overflow = (unsigned long long)-1;
    int any_digits = 0;

    while (1) {
        int digit;
        char character = *cursor;
        if (character >= '0' && character <= '9')
            digit = character - '0';
        else if (character >= 'a' && character <= 'z')
            digit = character - 'a' + 10;
        else if (character >= 'A' && character <= 'Z')
            digit = character - 'A' + 10;
        else
            break;

        if (digit >= base)
            break;

        // Overflow check: if result > (ULLONG_MAX - digit) / base, clamp.
        if (result > (overflow - (unsigned long long)digit) / (unsigned long long)base) {
            result = overflow;
        } else {
            result = result * (unsigned long long)base + (unsigned long long)digit;
        }
        any_digits = 1;
        cursor++;
    }

    if (endptr)
        *endptr = (char *)(any_digits ? cursor : str);

    return negative ? (~result + 1ULL) : result;  // two's-complement negation
}

// ---------------------------------------------------------------------------
// Signed variants — delegate to strtoull with clamping.
// ---------------------------------------------------------------------------

long long strtoll(const char *str, char **endptr, int base)
{
    const char *cursor = str;

    // Skip whitespace and detect sign before delegating to strtoull.
    // We must NOT pass the '-' to strtoull because strtoull applies
    // two's-complement negation itself, which would confuse our clamping.
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\n' ||
           *cursor == '\r' || *cursor == '\f' || *cursor == '\v')
        cursor++;

    int negative = 0;
    if (*cursor == '-') { negative = 1; cursor++; }
    else if (*cursor == '+') { cursor++; }

    // Parse the magnitude (unsigned, no sign).
    unsigned long long magnitude = strtoull(cursor, endptr, base);

    // LLONG_MAX = 0x7FFFFFFFFFFFFFFF, LLONG_MIN magnitude = 0x8000000000000000
    if (negative) {
        if (magnitude > 0x8000000000000000ULL)
            return (long long)0x8000000000000000ULL;  // clamp to LLONG_MIN
        return -(long long)magnitude;
    } else {
        if (magnitude > 0x7FFFFFFFFFFFFFFFULL)
            return (long long)0x7FFFFFFFFFFFFFFFULL;  // clamp to LLONG_MAX
        return (long long)magnitude;
    }
}

long strtol(const char *str, char **endptr, int base)
{
    long long result = strtoll(str, endptr, base);
    // LONG_MAX = 0x7FFFFFFF on 32-bit; on 64-bit AArch64, long is 64-bit.
    return (long)result;
}

unsigned long strtoul(const char *str, char **endptr, int base)
{
    return (unsigned long)strtoull(str, endptr, base);
}

// ---------------------------------------------------------------------------
// Convenience wrappers
// ---------------------------------------------------------------------------

int atoi(const char *str)
{
    return (int)strtol(str, (char **)0, 10);
}

long atol(const char *str)
{
    return strtol(str, (char **)0, 10);
}

long long atoll(const char *str)
{
    return strtoll(str, (char **)0, 10);
}

// ---------------------------------------------------------------------------
// Integer arithmetic
// ---------------------------------------------------------------------------

int abs(int n)
{
    return n < 0 ? -n : n;
}

long labs(long n)
{
    return n < 0 ? -n : n;
}

long long llabs(long long n)
{
    return n < 0 ? -n : n;
}

// ---------------------------------------------------------------------------
// abort — terminate immediately (no atexit, no flushing)
// ---------------------------------------------------------------------------

void abort(void)
{
    exit(134);  // 128 + SIGABRT(6) = conventional abort exit code
}

// ---------------------------------------------------------------------------
// Pseudo-random number generation — LCG (Numerical Recipes constants)
// ---------------------------------------------------------------------------

static unsigned int rand_state = 1;

int rand(void)
{
    rand_state = rand_state * 1664525u + 1013904223u;
    return (int)((rand_state >> 1) & 0x7fffffff);
}

void srand(unsigned int seed)
{
    rand_state = seed;
}

// ---------------------------------------------------------------------------
// bsearch — binary search (C11 §7.22.5.1)
// ---------------------------------------------------------------------------

void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compare)(const void *, const void *))
{
    size_t lo = 0, hi = nmemb;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const void *elem = (const char *)base + mid * size;
        int cmp = compare(key, elem);
        if (cmp == 0) return (void *)elem;
        if (cmp < 0)  hi = mid;
        else          lo = mid + 1;
    }
    return (void *)0;
}

// ---------------------------------------------------------------------------
// qsort — insertion sort for small arrays, recursive quicksort otherwise
// ---------------------------------------------------------------------------

static void swap(char *a, char *b, size_t size)
{
    while (size--) {
        char tmp = *a;
        *a++ = *b;
        *b++ = tmp;
    }
}

static void insertion_sort(char *base, size_t nmemb, size_t size,
                            int (*cmp)(const void *, const void *))
{
    for (size_t i = 1; i < nmemb; i++) {
        size_t j = i;
        while (j > 0 && cmp(base + (j-1)*size, base + j*size) > 0) {
            swap(base + (j-1)*size, base + j*size, size);
            j--;
        }
    }
}

void qsort(void *base, size_t nmemb, size_t size,
           int (*compare)(const void *, const void *))
{
    if (nmemb < 16) {
        insertion_sort((char *)base, nmemb, size, compare);
        return;
    }

    // Median-of-three pivot.
    size_t mid = nmemb / 2;
    char *b = (char *)base;
    if (compare(b, b + mid*size) > 0)            swap(b, b + mid*size, size);
    if (compare(b, b + (nmemb-1)*size) > 0)      swap(b, b + (nmemb-1)*size, size);
    if (compare(b + mid*size, b + (nmemb-1)*size) > 0)
        swap(b + mid*size, b + (nmemb-1)*size, size);

    // Place pivot at nmemb-2.
    swap(b + mid*size, b + (nmemb-2)*size, size);
    char *pivot = b + (nmemb-2)*size;

    size_t lo = 1, hi = nmemb - 2;
    while (1) {
        while (compare(b + lo*size, pivot) < 0) lo++;
        while (compare(b + hi*size, pivot) > 0) hi--;
        if (lo >= hi) break;
        swap(b + lo*size, b + hi*size, size);
        lo++; hi--;
    }
    swap(b + lo*size, pivot, size);

    qsort(b, lo, size, compare);
    qsort(b + (lo+1)*size, nmemb - lo - 1, size, compare);
}
