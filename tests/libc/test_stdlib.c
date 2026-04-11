// Tests for userspace/libc/stdlib.c
// Compile and run on the host: cc -o /tmp/test_stdlib tests/libc/test_stdlib.c userspace/libc/stdlib.c && /tmp/test_stdlib

#include "../../userspace/libc/stdlib.h"
#include <stdio.h>
#include <stdint.h>

static int passed = 0;
static int failed = 0;

#define TEST(desc, expr) do {                           \
    if (expr) {                                         \
        printf("  PASS  %s\n", desc);                   \
        passed++;                                       \
    } else {                                            \
        printf("  FAIL  %s  (line %d)\n", desc, __LINE__); \
        failed++;                                       \
    }                                                   \
} while (0)

// ---------------------------------------------------------------------------
// strtoull
// ---------------------------------------------------------------------------

static void test_strtoull(void)
{
    printf("\nstrtoull\n");
    TEST("decimal zero",           strtoull("0",    NULL, 10) == 0);
    TEST("decimal positive",       strtoull("42",   NULL, 10) == 42);
    TEST("decimal large",          strtoull("1000000", NULL, 10) == 1000000ULL);
    TEST("hex lowercase",          strtoull("ff",   NULL, 16) == 255);
    TEST("hex uppercase",          strtoull("FF",   NULL, 16) == 255);
    TEST("hex with 0x prefix",     strtoull("0xff", NULL, 16) == 255);
    TEST("hex with 0X prefix",     strtoull("0XFF", NULL, 16) == 255);
    TEST("base 0 detects hex",     strtoull("0xff", NULL,  0) == 255);
    TEST("base 0 detects octal",   strtoull("010",  NULL,  0) == 8);
    TEST("base 0 detects decimal", strtoull("42",   NULL,  0) == 42);
    TEST("base 2 binary",          strtoull("1010", NULL,  2) == 10);
    TEST("base 8 octal",           strtoull("17",   NULL,  8) == 15);
    TEST("leading whitespace",     strtoull("  42", NULL, 10) == 42);
    TEST("max uint64",             strtoull("18446744073709551615", NULL, 10) == (unsigned long long)-1);
    TEST("overflow clamps",        strtoull("99999999999999999999", NULL, 10) == (unsigned long long)-1);
    TEST("negative wraps",         strtoull("-1",   NULL, 10) == (unsigned long long)-1);

    // endptr
    char *end;
    strtoull("42abc", &end, 10);
    TEST("endptr stops at non-digit", *end == 'a');

    const char *no_digits = "xyz";
    strtoull(no_digits, &end, 10);
    TEST("endptr returns str on no digits", end == no_digits);
}

// ---------------------------------------------------------------------------
// strtoll
// ---------------------------------------------------------------------------

static void test_strtoll(void)
{
    printf("\nstrtoll\n");
    TEST("positive",        strtoll("42",   NULL, 10) ==  42);
    TEST("negative",        strtoll("-42",  NULL, 10) == -42);
    TEST("zero",            strtoll("0",    NULL, 10) ==  0);
    TEST("hex",             strtoll("0xff", NULL,  0) == 255);
    TEST("LLONG_MAX",       strtoll("9223372036854775807",  NULL, 10) ==  (long long)0x7FFFFFFFFFFFFFFFULL);
    TEST("LLONG_MIN",       strtoll("-9223372036854775808", NULL, 10) == (long long)0x8000000000000000ULL);
    TEST("overflow clamps to LLONG_MAX", strtoll("99999999999999999999", NULL, 10) == (long long)0x7FFFFFFFFFFFFFFFULL);
}

// ---------------------------------------------------------------------------
// strtol / strtoul
// ---------------------------------------------------------------------------

static void test_strtol(void)
{
    printf("\nstrtol / strtoul\n");
    TEST("strtol positive",  strtol("100",  NULL, 10) == 100);
    TEST("strtol negative",  strtol("-100", NULL, 10) == -100);
    TEST("strtoul positive", strtoul("255", NULL, 10) == 255);
    TEST("strtoul hex",      strtoul("ff",  NULL, 16) == 255);
}

// ---------------------------------------------------------------------------
// atoi / atol / atoll
// ---------------------------------------------------------------------------

static void test_atoi(void)
{
    printf("\natoi / atol / atoll\n");
    TEST("atoi zero",      atoi("0")    ==  0);
    TEST("atoi positive",  atoi("42")   ==  42);
    TEST("atoi negative",  atoi("-7")   == -7);
    TEST("atoi spaces",    atoi("  99") ==  99);
    TEST("atol positive",  atol("100")  == 100L);
    TEST("atoll positive", atoll("9999999999") == 9999999999LL);
}

// ---------------------------------------------------------------------------
// abs / labs / llabs
// ---------------------------------------------------------------------------

static void test_abs(void)
{
    printf("\nabs / labs / llabs\n");
    TEST("abs positive",  abs(5)    ==  5);
    TEST("abs negative",  abs(-5)   ==  5);
    TEST("abs zero",      abs(0)    ==  0);
    TEST("labs negative", labs(-1L) ==  1L);
    TEST("llabs negative",llabs(-9999999999LL) == 9999999999LL);
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

int main(void)
{
    printf("=== stdlib tests ===\n");
    test_strtoull();
    test_strtoll();
    test_strtol();
    test_atoi();
    test_abs();
    printf("\n%d passed, %d failed\n", passed, failed);
    return failed > 0 ? 1 : 0;
}
