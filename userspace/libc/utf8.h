#pragma once

#include <stddef.h>
#include <stdint.h>

// UTF-8 encoding/decoding utilities for Bazzulto OS.
// All strings in the system are UTF-8. These functions provide
// codepoint-level operations on top of the byte-level libc.

// Count the number of Unicode codepoints (visible characters) in a
// null-terminated UTF-8 string. Multi-byte sequences count as one.
// Invalid bytes are each counted as one codepoint.
// "café" → 4  (not 5)
size_t utf8_length(const char *str);

// Advance to the next codepoint in a UTF-8 string.
// Returns a pointer to the start of the next codepoint,
// or to the null terminator if at the end.
const char *utf8_next(const char *str);

// Retreat to the previous codepoint in a UTF-8 string.
// `start` is the beginning of the string (to avoid underflow).
// Returns a pointer to the start of the previous codepoint,
// or `start` if already at the beginning.
const char *utf8_prev(const char *str, const char *start);

// Decode one codepoint from a UTF-8 string.
// Advances `*str` past the consumed bytes.
// Returns the Unicode codepoint value (U+0000 to U+10FFFF).
// Returns 0xFFFD (replacement character) on invalid input.
uint32_t utf8_decode(const char **str);

// Encode a Unicode codepoint as UTF-8 into `buf`.
// `buf` must have room for at least 4 bytes.
// Returns the number of bytes written (1–4), or 0 if the
// codepoint is invalid (> U+10FFFF).
int utf8_encode(uint32_t codepoint, char *buf);

// Validate that `len` bytes starting at `str` are well-formed UTF-8.
// Returns 1 if valid, 0 if any invalid sequence is found.
int utf8_validate(const char *str, size_t len);

// Return the number of bytes used by the first codepoint in `str`.
// Returns 1–4 for valid sequences, or 1 for invalid lead bytes.
int utf8_codepoint_size(const char *str);
