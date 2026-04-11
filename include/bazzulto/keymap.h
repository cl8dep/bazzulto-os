#pragma once

#include <stddef.h>
#include <stdint.h>

// ---------------------------------------------------------------------------
// .bkm (Bazzulto KeyMap) parser
//
// Parses a .bkm file into a lookup table that translates evdev keycodes
// + modifier state into UTF-8 character sequences.
//
// File format: plain text, one mapping per line.
//   evdev_code  normal  shift  altgr
//
// Special escapes: \n \t \b \s \e  (enter, tab, backspace, space, escape)
// No mapping: -
// Dead keys: DEAD_ACUTE, DEAD_GRAVE, DEAD_CIRCUMFLEX, DEAD_TILDE, DEAD_DIAERESIS
// Named keys: UP, DOWN, LEFT, RIGHT, F1-F12, INSERT, DELETE, HOME, END, PGUP, PGDN
// Comments: lines starting with #
// ---------------------------------------------------------------------------

// Maximum evdev keycode supported.
#define KEYMAP_MAX_EVDEV_CODE   128

// Maximum UTF-8 bytes per key mapping (4 bytes for a codepoint + NUL).
#define KEYMAP_MAX_CHAR_BYTES   5

// Modifier indices for the keymap table.
#define KEYMAP_MODIFIER_NORMAL      0
#define KEYMAP_MODIFIER_SHIFT       1
#define KEYMAP_MODIFIER_ALTGR       2
#define KEYMAP_MODIFIER_SHIFT_ALTGR 3
#define KEYMAP_MODIFIER_COUNT       4

// Special key codes returned in the first byte of the mapping.
// These are outside the UTF-8 range and indicate non-character keys.
#define KEYMAP_SPECIAL_NONE     0x00
#define KEYMAP_SPECIAL_UP       0x01
#define KEYMAP_SPECIAL_DOWN     0x02
#define KEYMAP_SPECIAL_LEFT     0x03
#define KEYMAP_SPECIAL_RIGHT    0x04
#define KEYMAP_SPECIAL_INSERT   0x05
#define KEYMAP_SPECIAL_DELETE   0x06
#define KEYMAP_SPECIAL_HOME     0x07
#define KEYMAP_SPECIAL_END      0x08
#define KEYMAP_SPECIAL_PGUP     0x09
#define KEYMAP_SPECIAL_PGDN     0x0A
#define KEYMAP_SPECIAL_F1       0x10
// F2..F12 = 0x11..0x1B

// Dead key markers. Stored as first byte when the mapping is a dead key.
#define KEYMAP_DEAD_ACUTE       0xE0
#define KEYMAP_DEAD_GRAVE       0xE1
#define KEYMAP_DEAD_CIRCUMFLEX  0xE2
#define KEYMAP_DEAD_TILDE       0xE3
#define KEYMAP_DEAD_DIAERESIS   0xE4

// The keymap table: [evdev_code][modifier] → UTF-8 bytes (NUL-terminated).
// A mapping of all zeros means no mapping for that key+modifier.
typedef struct {
    char table[KEYMAP_MAX_EVDEV_CODE][KEYMAP_MODIFIER_COUNT][KEYMAP_MAX_CHAR_BYTES];
} keymap_t;

// Parse a .bkm file from a NUL-terminated string into a keymap_t.
// Returns 0 on success, -1 on parse error.
// The keymap is zeroed before parsing — unmapped keys produce empty strings.
int keymap_parse(const char *bkm_data, size_t bkm_length, keymap_t *out);

// Look up a character for a given evdev keycode and modifier state.
// Returns a pointer to the NUL-terminated UTF-8 string (may be empty if unmapped).
const char *keymap_lookup(const keymap_t *keymap, uint16_t evdev_code, int modifier);

// Check if a mapping is a dead key.
// Returns the KEYMAP_DEAD_* constant, or 0 if not a dead key.
uint8_t keymap_is_dead_key(const char *mapping);

// Resolve a dead key + base character into a composed UTF-8 character.
// Writes the result into `out` (must have room for KEYMAP_MAX_CHAR_BYTES).
// Returns the number of bytes written, or 0 if no compose rule exists
// (in which case the dead key should be emitted literally).
int keymap_compose_dead_key(uint8_t dead_key, const char *base, char *out);

// Get the literal UTF-8 character for a dead key (e.g. DEAD_ACUTE → "'").
// Used when a dead key has no compose match and needs to be emitted as-is.
// Writes into `out` (must have room for KEYMAP_MAX_CHAR_BYTES).
// Returns the number of bytes written.
int keymap_dead_key_literal(uint8_t dead_key, char *out);
