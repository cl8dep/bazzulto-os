#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// 8x8 bitmap font covering ASCII (U+0020..U+007F) and Latin Extended
// characters commonly used in European languages.
//
// The font is stored as a flat array of 8-byte glyph bitmaps.
// Each glyph is 8 rows of 8 pixels, 1 bit per pixel, LSB is leftmost.
// Source: public domain font based on IBM CP437, extended with Latin-1
// glyphs generated from the CP437/CP850 character sets.
//
// Lookup: use font_latin_extended_lookup(codepoint) to get a pointer to the
// 8-byte glyph, or NULL if the codepoint has no glyph.
// ---------------------------------------------------------------------------

// Returns a pointer to the 8-byte glyph bitmap for the given Unicode
// codepoint, or NULL if the codepoint is not in the font.
const uint8_t *font_latin_extended_lookup(uint32_t codepoint);
