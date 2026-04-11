// Tests for userspace/libc/utf8.c
// Compile and run on the host: cc -o /tmp/test_utf8 tests/libc/test_utf8.c userspace/libc/utf8.c && /tmp/test_utf8

#include "../../userspace/libc/utf8.h"
#include <stdio.h>
#include <string.h>

static int passed = 0;
static int failed = 0;

#define TEST(desc, expr) do {                               \
    if (expr) {                                             \
        printf("  PASS  %s\n", desc);                       \
        passed++;                                           \
    } else {                                                \
        printf("  FAIL  %s  (line %d)\n", desc, __LINE__); \
        failed++;                                           \
    }                                                       \
} while (0)

// ---------------------------------------------------------------------------
// utf8_codepoint_size
// ---------------------------------------------------------------------------

static void test_codepoint_size(void)
{
    printf("\nutf8_codepoint_size\n");
    TEST("ASCII 'A' is 1 byte",       utf8_codepoint_size("A") == 1);
    TEST("ASCII NUL is 1 byte",        utf8_codepoint_size("\0") == 1);
    TEST("2-byte: e-acute",            utf8_codepoint_size("\xC3\xA9") == 2);
    TEST("3-byte: euro sign",          utf8_codepoint_size("\xE2\x82\xAC") == 3);
    TEST("4-byte: smiley",             utf8_codepoint_size("\xF0\x9F\x98\x80") == 4);
    TEST("invalid lead 0xFF is 1",     utf8_codepoint_size("\xFF") == 1);
    TEST("continuation 0x80 is 1",     utf8_codepoint_size("\x80") == 1);
}

// ---------------------------------------------------------------------------
// utf8_decode
// ---------------------------------------------------------------------------

static void test_decode(void)
{
    printf("\nutf8_decode\n");
    const char *s;

    s = "A";
    TEST("decode ASCII 'A'",           utf8_decode(&s) == 0x41 && *s == '\0');

    s = "\xC3\xA9";  // é = U+00E9
    TEST("decode e-acute",             utf8_decode(&s) == 0xE9 && *s == '\0');

    s = "\xC3\xB1";  // ñ = U+00F1
    TEST("decode n-tilde",             utf8_decode(&s) == 0xF1 && *s == '\0');

    s = "\xE2\x82\xAC";  // € = U+20AC
    TEST("decode euro sign",           utf8_decode(&s) == 0x20AC && *s == '\0');

    s = "\xF0\x9F\x98\x80";  // 😀 = U+1F600
    TEST("decode smiley",              utf8_decode(&s) == 0x1F600 && *s == '\0');

    // Overlong encoding of '/' (U+002F) as 2 bytes: 0xC0 0xAF
    s = "\xC0\xAF";
    TEST("reject overlong 2-byte",     utf8_decode(&s) == 0xFFFD);

    // Surrogate half U+D800
    s = "\xED\xA0\x80";
    TEST("reject surrogate",           utf8_decode(&s) == 0xFFFD);

    // Invalid lead byte
    s = "\xFF";
    TEST("reject 0xFF",                utf8_decode(&s) == 0xFFFD);

    // Truncated 2-byte: lead only, then NUL
    s = "\xC3";
    TEST("reject truncated 2-byte",    utf8_decode(&s) == 0xFFFD);
}

// ---------------------------------------------------------------------------
// utf8_encode
// ---------------------------------------------------------------------------

static void test_encode(void)
{
    printf("\nutf8_encode\n");
    char buf[4];
    int len;

    len = utf8_encode(0x41, buf);  // 'A'
    TEST("encode ASCII 1 byte",        len == 1 && buf[0] == 'A');

    len = utf8_encode(0xE9, buf);  // é
    TEST("encode e-acute 2 bytes",     len == 2 && (unsigned char)buf[0] == 0xC3 && (unsigned char)buf[1] == 0xA9);

    len = utf8_encode(0xF1, buf);  // ñ
    TEST("encode n-tilde 2 bytes",     len == 2 && (unsigned char)buf[0] == 0xC3 && (unsigned char)buf[1] == 0xB1);

    len = utf8_encode(0x20AC, buf);  // €
    TEST("encode euro 3 bytes",        len == 3 && (unsigned char)buf[0] == 0xE2 && (unsigned char)buf[1] == 0x82 && (unsigned char)buf[2] == 0xAC);

    len = utf8_encode(0x1F600, buf);  // 😀
    TEST("encode smiley 4 bytes",      len == 4 && (unsigned char)buf[0] == 0xF0);

    len = utf8_encode(0xD800, buf);  // surrogate
    TEST("reject surrogate",           len == 0);

    len = utf8_encode(0x110000, buf);  // out of range
    TEST("reject > U+10FFFF",          len == 0);
}

// ---------------------------------------------------------------------------
// utf8_length
// ---------------------------------------------------------------------------

static void test_length(void)
{
    printf("\nutf8_length\n");
    TEST("empty string",               utf8_length("") == 0);
    TEST("ASCII 'hello'",              utf8_length("hello") == 5);
    TEST("'cafe' with e-acute",        utf8_length("caf\xC3\xA9") == 4);
    TEST("euro sign alone",            utf8_length("\xE2\x82\xAC") == 1);
    TEST("smiley alone",               utf8_length("\xF0\x9F\x98\x80") == 1);
    TEST("mixed ASCII + multi-byte",   utf8_length("a\xC3\xB1" "o") == 3);  // 'año' = 3
}

// ---------------------------------------------------------------------------
// utf8_next / utf8_prev
// ---------------------------------------------------------------------------

static void test_navigation(void)
{
    printf("\nutf8_next\n");
    const char *s = "a\xC3\xB1o";  // 'año'

    const char *p = s;
    TEST("next from 'a'",             utf8_next(p) == s + 1);
    p = s + 1;
    TEST("next from 'ñ'",             utf8_next(p) == s + 3);  // ñ is 2 bytes
    p = s + 3;
    TEST("next from 'o'",             utf8_next(p) == s + 4);
    p = s + 4;
    TEST("next from NUL stays",        utf8_next(p) == s + 4);

    printf("\nutf8_prev\n");
    p = s + 4;  // at NUL
    TEST("prev from end to 'o'",       utf8_prev(p, s) == s + 3);
    p = s + 3;
    TEST("prev from 'o' to 'ñ'",       utf8_prev(p, s) == s + 1);
    p = s + 1;
    TEST("prev from 'ñ' to 'a'",       utf8_prev(p, s) == s);
    p = s;
    TEST("prev from start stays",      utf8_prev(p, s) == s);
}

// ---------------------------------------------------------------------------
// utf8_validate
// ---------------------------------------------------------------------------

static void test_validate(void)
{
    printf("\nutf8_validate\n");
    TEST("valid ASCII",                utf8_validate("hello", 5) == 1);
    TEST("valid empty",                utf8_validate("", 0) == 1);
    TEST("valid 2-byte",               utf8_validate("\xC3\xA9", 2) == 1);
    TEST("valid 3-byte",               utf8_validate("\xE2\x82\xAC", 3) == 1);
    TEST("valid 4-byte",               utf8_validate("\xF0\x9F\x98\x80", 4) == 1);
    TEST("valid mixed",                utf8_validate("caf\xC3\xA9", 5) == 1);
    TEST("invalid lone continuation",  utf8_validate("\x80", 1) == 0);
    TEST("invalid truncated",          utf8_validate("\xC3", 1) == 0);
    TEST("invalid overlong",           utf8_validate("\xC0\xAF", 2) == 0);
    TEST("invalid surrogate",          utf8_validate("\xED\xA0\x80", 3) == 0);
    TEST("invalid 0xFF",               utf8_validate("\xFF", 1) == 0);
}

// ---------------------------------------------------------------------------
// roundtrip: encode → decode
// ---------------------------------------------------------------------------

static void test_roundtrip(void)
{
    printf("\nroundtrip encode/decode\n");
    uint32_t codepoints[] = { 0x41, 0xE9, 0xF1, 0x20AC, 0x1F600, 0x00, 0x7F };
    int count = (int)(sizeof(codepoints) / sizeof(codepoints[0]));

    for (int i = 0; i < count; i++) {
        char buf[4];
        int len = utf8_encode(codepoints[i], buf);
        const char *p = buf;
        uint32_t decoded = utf8_decode(&p);
        char desc[64];
        snprintf(desc, sizeof(desc), "U+%04X roundtrips", codepoints[i]);
        TEST(desc, decoded == codepoints[i] && (int)(p - buf) == len);
    }
}

// ---------------------------------------------------------------------------

int main(void)
{
    printf("=== utf8 tests ===\n");
    test_codepoint_size();
    test_decode();
    test_encode();
    test_length();
    test_navigation();
    test_validate();
    test_roundtrip();
    printf("\n%d passed, %d failed\n", passed, failed);
    return failed ? 1 : 0;
}
