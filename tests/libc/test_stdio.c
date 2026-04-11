// Tests for userspace/libc/stdio.c — snprintf / vsnprintf
//
// Compile on macOS ARM64:
//   cc -arch arm64 -std=c11 \
//      -I userspace/library \
//      -o /tmp/test_stdio \
//      tests/libc/test_stdio.c userspace/libc/stdio_test_shim.c userspace/libc/string.c \
//   && /tmp/test_stdio

// This file uses ONLY the host <stdio.h> / <string.h> for test output.
// Our libc functions are accessed via bz_* forward declarations (from the shim)
// so we never include our conflicting stdio.h here.

#include <stdio.h>
#include <string.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>
#include <unistd.h>   // syscall write — bypasses the stubbed write in the shim

// Forward declarations of our libc functions (implemented in stdio_test_shim.c).
int bz_snprintf(char *buf, size_t size, const char *fmt, ...);
int bz_vsnprintf(char *buf, size_t size, const char *fmt, va_list ap);

// Use the real system write so test output reaches the terminal.
// The shim's write() stub only affects code linked from stdio.c.
#define PRINT(s) ((void)write(STDOUT_FILENO, (s), __builtin_strlen(s)))

static int passed = 0;
static int failed = 0;

static void report(const char *desc, int ok, int line)
{
    char buf[256];
    if (ok) {
        bz_snprintf(buf, sizeof(buf), "  PASS  %s\n", desc);
    } else {
        bz_snprintf(buf, sizeof(buf), "  FAIL  %s  (line %d)\n", desc, line);
    }
    // Use the real syscall write — not the stub.
    write(STDOUT_FILENO, buf, strlen(buf));
    if (ok) passed++; else failed++;
}

#define TEST(desc, expr) report(desc, (expr), __LINE__)

static void test_snprintf(void)
{
    char buf[64];

    PRINT("\nsnprintf -- conversions\n");

    bz_snprintf(buf, sizeof(buf), "%s", "hello");
    TEST("%s basic",            strcmp(buf, "hello") == 0);

    bz_snprintf(buf, sizeof(buf), "%s", (char *)0);
    TEST("%s null",             strcmp(buf, "(null)") == 0);

    bz_snprintf(buf, sizeof(buf), "%c", 'A');
    TEST("%c",                  buf[0] == 'A' && buf[1] == '\0');

    bz_snprintf(buf, sizeof(buf), "%d", 42);
    TEST("%d positive",         strcmp(buf, "42") == 0);

    bz_snprintf(buf, sizeof(buf), "%d", -7);
    TEST("%d negative",         strcmp(buf, "-7") == 0);

    bz_snprintf(buf, sizeof(buf), "%d", 0);
    TEST("%d zero",             strcmp(buf, "0") == 0);

    bz_snprintf(buf, sizeof(buf), "%i", 99);
    TEST("%i",                  strcmp(buf, "99") == 0);

    bz_snprintf(buf, sizeof(buf), "%u", 4294967295u);
    TEST("%u max uint32",       strcmp(buf, "4294967295") == 0);

    bz_snprintf(buf, sizeof(buf), "%x", 255u);
    TEST("%x hex lower",        strcmp(buf, "ff") == 0);

    bz_snprintf(buf, sizeof(buf), "%X", 255u);
    TEST("%X hex upper",        strcmp(buf, "FF") == 0);

    bz_snprintf(buf, sizeof(buf), "%%");
    TEST("%%%%",                strcmp(buf, "%") == 0);

    bz_snprintf(buf, sizeof(buf), "%lld", (long long)-9223372036854775807LL - 1LL);
    TEST("%lld LLONG_MIN",      strcmp(buf, "-9223372036854775808") == 0);

    bz_snprintf(buf, sizeof(buf), "%llu", (unsigned long long)18446744073709551615ULL);
    TEST("%llu ULLONG_MAX",     strcmp(buf, "18446744073709551615") == 0);

    bz_snprintf(buf, sizeof(buf), "%llx", (unsigned long long)0xdeadbeefcafeULL);
    TEST("%llx",                strcmp(buf, "deadbeefcafe") == 0);

    PRINT("\nsnprintf -- width / left-justify\n");

    bz_snprintf(buf, sizeof(buf), "%5d", 42);
    TEST("right-pad width",     strcmp(buf, "   42") == 0);

    bz_snprintf(buf, sizeof(buf), "%-5d|", 42);
    TEST("left-justify",        strcmp(buf, "42   |") == 0);

    bz_snprintf(buf, sizeof(buf), "%5s", "hi");
    TEST("right-pad string",    strcmp(buf, "   hi") == 0);

    PRINT("\nsnprintf -- truncation\n");

    int ret = bz_snprintf(buf, 4, "%s", "hello");
    TEST("truncates to size-1", strcmp(buf, "hel") == 0);
    TEST("returns chars written", ret == 3);

    bz_snprintf(buf, sizeof(buf), "%s", "");
    TEST("empty string",        buf[0] == '\0');

    PRINT("\nsnprintf -- mixed\n");

    bz_snprintf(buf, sizeof(buf), "%s=%d", "answer", 42);
    TEST("mixed format",        strcmp(buf, "answer=42") == 0);
}

int main(void)
{
    PRINT("=== stdio tests ===\n");
    test_snprintf();

    char summary[64];
    bz_snprintf(summary, sizeof(summary), "\n%d passed, %d failed\n", passed, failed);
    write(STDOUT_FILENO, summary, strlen(summary));
    return failed > 0 ? 1 : 0;
}
