// Shim that compiles userspace/libc/stdio.c for the host test environment.
// Exports bz_snprintf / bz_vsnprintf for use by test_stdio.c.
//
// Problem: stdio.c calls write(1, buf, len) to emit output. In tests we want
// to suppress that output (printf/puts side effects), but we must NOT intercept
// the write() calls that test_stdio.c uses for its own output.
//
// Solution: compile stdio.c with write renamed to bz_write_stub via -D, so
// only code inside stdio.c uses the stub, not the linker-global write symbol.

#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>

// Internal stub — only used by the stdio.c code compiled below.
// Named differently from write() so it doesn't intercept the test's output.
static int64_t bz_write_stub(int fd, const char *buf, size_t len)
{
    (void)fd; (void)buf; (void)len;
    return (int64_t)len;
}

// Redirect write → bz_write_stub for the stdio.c translation unit only.
#define write bz_write_stub
#include "stdio.c"
#undef write

// Re-export under bz_ names for test_stdio.c.
int bz_snprintf(char *buf, size_t size, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return r;
}

int bz_vsnprintf(char *buf, size_t size, const char *fmt, va_list ap)
{
    return vsnprintf(buf, size, fmt, ap);
}
