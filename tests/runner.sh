#!/bin/sh
# Run all libc tests on the host machine (macOS ARM64 / Linux ARM64).
# Usage: ./tests/runner.sh
# Returns exit code 1 if any test suite fails.

set -e
cd "$(dirname "$0")/.."

CC="${CC:-cc}"
ARCH_FLAG="-arch arm64"

# Detect if -arch arm64 is supported (macOS only).
if ! $CC $ARCH_FLAG -x c -o /dev/null - < /dev/null 2>/dev/null; then
    ARCH_FLAG=""
fi

CFLAGS="$ARCH_FLAG -std=c11 -Wall -Wno-builtin-requires-header"
FAILED=0

run_test() {
    name="$1"
    binary="$2"
    shift 2
    printf "\n──────────────────────────────────────────\n"
    printf "Running: %s\n" "$name"
    printf "──────────────────────────────────────────\n"
    $CC $CFLAGS "$@" -o "$binary" && "$binary" || FAILED=1
}

run_test "string" /tmp/bz_test_string \
    tests/libc/test_string.c \
    userspace/libc/string.c

run_test "stdlib" /tmp/bz_test_stdlib \
    -I userspace/libc \
    -DBAZZULTO_HOST_TEST \
    tests/libc/test_stdlib.c \
    userspace/libc/stdlib.c

run_test "stdio" /tmp/bz_test_stdio \
    -I userspace/library \
    tests/libc/test_stdio.c \
    userspace/libc/stdio_test_shim.c \
    userspace/libc/string.c

run_test "utf8" /tmp/bz_test_utf8 \
    tests/libc/test_utf8.c \
    userspace/libc/utf8.c

run_test "ctype" /tmp/bz_test_ctype \
    tests/libc/test_ctype.c \
    userspace/libc/ctype.c

run_test "stdlib_extended" /tmp/bz_test_stdlib_ext \
    -I userspace/libc \
    -DBAZZULTO_HOST_TEST \
    tests/libc/test_stdlib_extended.c \
    userspace/libc/stdlib.c \
    userspace/libc/string.c

printf "\n──────────────────────────────────────────\n"
if [ "$FAILED" -eq 0 ]; then
    printf "All test suites passed.\n"
else
    printf "One or more test suites FAILED.\n"
fi
printf "──────────────────────────────────────────\n"

exit $FAILED
