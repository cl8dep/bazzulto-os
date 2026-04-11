#include "../../include/libc/stdlib.h"

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

    // Determine sign so we can clamp correctly.
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\n' ||
           *cursor == '\r' || *cursor == '\f' || *cursor == '\v')
        cursor++;

    int negative = 0;
    if (*cursor == '-') negative = 1;

    unsigned long long raw = strtoull(str, endptr, base);

    // LLONG_MAX = 0x7FFFFFFFFFFFFFFF, LLONG_MIN = 0x8000000000000000
    if (negative) {
        if (raw > 0x8000000000000000ULL)
            return (long long)0x8000000000000000ULL;  // LLONG_MIN
        return -(long long)raw;
    } else {
        if (raw > 0x7FFFFFFFFFFFFFFFULL)
            return (long long)0x7FFFFFFFFFFFFFFFULL;  // LLONG_MAX
        return (long long)raw;
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
