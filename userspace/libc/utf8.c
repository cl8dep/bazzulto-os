#include "utf8.h"

// ---------------------------------------------------------------------------
// UTF-8 encoding reference (RFC 3629):
//
//   Codepoint range       Byte 1     Byte 2     Byte 3     Byte 4
//   U+0000..U+007F        0xxxxxxx
//   U+0080..U+07FF        110xxxxx   10xxxxxx
//   U+0800..U+FFFF        1110xxxx   10xxxxxx   10xxxxxx
//   U+10000..U+10FFFF     11110xxx   10xxxxxx   10xxxxxx   10xxxxxx
//
// Continuation bytes always match 10xxxxxx (0x80..0xBF).
// ---------------------------------------------------------------------------

static int is_continuation(unsigned char byte)
{
    return (byte & 0xC0) == 0x80;
}

int utf8_codepoint_size(const char *str)
{
    unsigned char lead = (unsigned char)*str;
    if (lead < 0x80) return 1;           // 0xxxxxxx — ASCII
    if ((lead & 0xE0) == 0xC0) return 2; // 110xxxxx
    if ((lead & 0xF0) == 0xE0) return 3; // 1110xxxx
    if ((lead & 0xF8) == 0xF0) return 4; // 11110xxx
    return 1;                             // invalid lead byte — treat as 1
}

uint32_t utf8_decode(const char **str)
{
    unsigned char lead = (unsigned char)**str;
    uint32_t codepoint;
    int expected;

    if (lead < 0x80) {
        (*str)++;
        return lead;
    }

    if ((lead & 0xE0) == 0xC0) {
        codepoint = lead & 0x1F;
        expected = 1;
    } else if ((lead & 0xF0) == 0xE0) {
        codepoint = lead & 0x0F;
        expected = 2;
    } else if ((lead & 0xF8) == 0xF0) {
        codepoint = lead & 0x07;
        expected = 3;
    } else {
        // Invalid lead byte.
        (*str)++;
        return 0xFFFD;
    }

    (*str)++;
    for (int i = 0; i < expected; i++) {
        if (!is_continuation((unsigned char)**str)) {
            return 0xFFFD; // truncated sequence — don't advance past the non-continuation
        }
        codepoint = (codepoint << 6) | ((unsigned char)**str & 0x3F);
        (*str)++;
    }

    // Reject overlong encodings and surrogates.
    if (codepoint <= 0x7F && expected >= 1) return 0xFFFD;
    if (codepoint <= 0x7FF && expected >= 2) return 0xFFFD;
    if (codepoint <= 0xFFFF && expected >= 3) return 0xFFFD;
    if (codepoint >= 0xD800 && codepoint <= 0xDFFF) return 0xFFFD;
    if (codepoint > 0x10FFFF) return 0xFFFD;

    return codepoint;
}

int utf8_encode(uint32_t codepoint, char *buf)
{
    if (codepoint <= 0x7F) {
        buf[0] = (char)codepoint;
        return 1;
    }
    if (codepoint <= 0x7FF) {
        buf[0] = (char)(0xC0 | (codepoint >> 6));
        buf[1] = (char)(0x80 | (codepoint & 0x3F));
        return 2;
    }
    if (codepoint <= 0xFFFF) {
        if (codepoint >= 0xD800 && codepoint <= 0xDFFF) return 0; // surrogates
        buf[0] = (char)(0xE0 | (codepoint >> 12));
        buf[1] = (char)(0x80 | ((codepoint >> 6) & 0x3F));
        buf[2] = (char)(0x80 | (codepoint & 0x3F));
        return 3;
    }
    if (codepoint <= 0x10FFFF) {
        buf[0] = (char)(0xF0 | (codepoint >> 18));
        buf[1] = (char)(0x80 | ((codepoint >> 12) & 0x3F));
        buf[2] = (char)(0x80 | ((codepoint >> 6) & 0x3F));
        buf[3] = (char)(0x80 | (codepoint & 0x3F));
        return 4;
    }
    return 0; // invalid codepoint
}

const char *utf8_next(const char *str)
{
    if (*str == '\0') return str;
    int size = utf8_codepoint_size(str);
    // Verify continuation bytes exist before skipping.
    for (int i = 1; i < size; i++) {
        if (str[i] == '\0' || !is_continuation((unsigned char)str[i]))
            return str + i;
    }
    return str + size;
}

const char *utf8_prev(const char *str, const char *start)
{
    if (str <= start) return start;
    str--;
    // Walk back over continuation bytes (max 3).
    int count = 0;
    while (str > start && is_continuation((unsigned char)*str) && count < 3) {
        str--;
        count++;
    }
    return str;
}

size_t utf8_length(const char *str)
{
    size_t count = 0;
    while (*str) {
        str = utf8_next(str);
        count++;
    }
    return count;
}

int utf8_validate(const char *str, size_t len)
{
    const char *end = str + len;
    while (str < end) {
        unsigned char lead = (unsigned char)*str;

        int expected_size;
        uint32_t minimum;

        if (lead < 0x80) {
            str++;
            continue;
        } else if ((lead & 0xE0) == 0xC0) {
            expected_size = 2;
            minimum = 0x80;
        } else if ((lead & 0xF0) == 0xE0) {
            expected_size = 3;
            minimum = 0x800;
        } else if ((lead & 0xF8) == 0xF0) {
            expected_size = 4;
            minimum = 0x10000;
        } else {
            return 0; // invalid lead byte
        }

        if (str + expected_size > end) return 0; // truncated

        // Verify all continuation bytes.
        for (int i = 1; i < expected_size; i++) {
            if (!is_continuation((unsigned char)str[i])) return 0;
        }

        // Decode and check overlong / surrogate / out-of-range.
        const char *tmp = str;
        uint32_t codepoint = utf8_decode(&tmp);
        if (codepoint == 0xFFFD) return 0;
        if (codepoint < minimum) return 0;

        str += expected_size;
    }
    return 1;
}
