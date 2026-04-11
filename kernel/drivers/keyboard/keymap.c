#include "../../../include/bazzulto/keymap.h"
#include "../../lib/utf8.h"

// ---------------------------------------------------------------------------
// .bkm parser — translates a plain-text keymap file into a lookup table.
// ---------------------------------------------------------------------------

// Forward declarations of helpers.
static void skip_whitespace(const char **cursor);
static void skip_line(const char **cursor);
static int parse_int(const char **cursor);
static int parse_token(const char **cursor, char *out, size_t out_size);
static int resolve_escape(const char *token, char *out);
static int resolve_named_key(const char *token, char *out);
static int resolve_dead_key(const char *token, char *out);

// String comparison (avoid dependency on string.h in kernel context).
static int str_eq(const char *a, const char *b)
{
    while (*a && *b) {
        if (*a != *b) return 0;
        a++; b++;
    }
    return *a == *b;
}

static int str_len(const char *s)
{
    int len = 0;
    while (s[len]) len++;
    return len;
}

static void mem_zero(void *dst, size_t n)
{
    char *d = (char *)dst;
    while (n--) *d++ = 0;
}

static void mem_copy(void *dst, const void *src, size_t n)
{
    char *d = (char *)dst;
    const char *s = (const char *)src;
    while (n--) *d++ = *s++;
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

static void skip_whitespace(const char **cursor)
{
    while (**cursor == ' ' || **cursor == '\t') (*cursor)++;
}

static void skip_line(const char **cursor)
{
    while (**cursor && **cursor != '\n') (*cursor)++;
    if (**cursor == '\n') (*cursor)++;
}

static int parse_int(const char **cursor)
{
    int value = 0;
    while (**cursor >= '0' && **cursor <= '9') {
        value = value * 10 + (**cursor - '0');
        (*cursor)++;
    }
    return value;
}

// Read a whitespace-delimited token into out. Returns token length, 0 if EOL/EOF.
static int parse_token(const char **cursor, char *out, size_t out_size)
{
    skip_whitespace(cursor);
    if (**cursor == '\0' || **cursor == '\n' || **cursor == '#')
        return 0;

    size_t len = 0;
    while (**cursor && **cursor != ' ' && **cursor != '\t' &&
           **cursor != '\n' && **cursor != '#' && len < out_size - 1) {
        out[len++] = **cursor;
        (*cursor)++;
    }
    out[len] = '\0';
    return (int)len;
}

// Resolve a single-character escape like \n, \t, \b, \s, \e.
// Returns bytes written to out (1), or 0 if not an escape.
static int resolve_escape(const char *token, char *out)
{
    if (token[0] != '\\' || token[1] == '\0' || token[2] != '\0')
        return 0;

    switch (token[1]) {
        case 'n': out[0] = '\n'; out[1] = '\0'; return 1;
        case 't': out[0] = '\t'; out[1] = '\0'; return 1;
        case 'b': out[0] = '\x7F'; out[1] = '\0'; return 1;  // DEL for backspace
        case 's': out[0] = ' ';  out[1] = '\0'; return 1;
        case 'e': out[0] = '\x1B'; out[1] = '\0'; return 1;  // ESC
        case '\\': out[0] = '\\'; out[1] = '\0'; return 1;
        default: return 0;
    }
}

// Resolve named special keys (UP, DOWN, F1, etc.).
// Returns 1 if resolved, 0 if not a named key.
static int resolve_named_key(const char *token, char *out)
{
    struct { const char *name; uint8_t code; } named_keys[] = {
        { "UP",     KEYMAP_SPECIAL_UP },
        { "DOWN",   KEYMAP_SPECIAL_DOWN },
        { "LEFT",   KEYMAP_SPECIAL_LEFT },
        { "RIGHT",  KEYMAP_SPECIAL_RIGHT },
        { "INSERT", KEYMAP_SPECIAL_INSERT },
        { "DELETE", KEYMAP_SPECIAL_DELETE },
        { "HOME",   KEYMAP_SPECIAL_HOME },
        { "END",    KEYMAP_SPECIAL_END },
        { "PGUP",   KEYMAP_SPECIAL_PGUP },
        { "PGDN",   KEYMAP_SPECIAL_PGDN },
    };

    for (int i = 0; i < (int)(sizeof(named_keys) / sizeof(named_keys[0])); i++) {
        if (str_eq(token, named_keys[i].name)) {
            out[0] = (char)named_keys[i].code;
            out[1] = '\0';
            return 1;
        }
    }

    // Function keys: F1..F12
    if (token[0] == 'F' && token[1] >= '1' && token[1] <= '9') {
        int fn;
        if (token[2] == '\0') {
            fn = token[1] - '0';  // F1..F9
        } else if (token[1] == '1' && token[2] >= '0' && token[2] <= '2' && token[3] == '\0') {
            fn = 10 + (token[2] - '0');  // F10..F12
        } else {
            return 0;
        }
        out[0] = (char)(KEYMAP_SPECIAL_F1 + fn - 1);
        out[1] = '\0';
        return 1;
    }

    return 0;
}

// Resolve dead key names.
// Returns 1 if resolved, 0 if not a dead key.
static int resolve_dead_key(const char *token, char *out)
{
    struct { const char *name; uint8_t code; } dead_keys[] = {
        { "DEAD_ACUTE",       KEYMAP_DEAD_ACUTE },
        { "DEAD_GRAVE",       KEYMAP_DEAD_GRAVE },
        { "DEAD_CIRCUMFLEX",  KEYMAP_DEAD_CIRCUMFLEX },
        { "DEAD_TILDE",       KEYMAP_DEAD_TILDE },
        { "DEAD_DIAERESIS",   KEYMAP_DEAD_DIAERESIS },
    };

    for (int i = 0; i < (int)(sizeof(dead_keys) / sizeof(dead_keys[0])); i++) {
        if (str_eq(token, dead_keys[i].name)) {
            out[0] = (char)dead_keys[i].code;
            out[1] = '\0';
            return 1;
        }
    }
    return 0;
}

// Resolve a token into a character mapping. Writes NUL-terminated UTF-8
// (or special code) into out. Returns bytes written (not counting NUL), or -1 on error.
static int resolve_token(const char *token, char *out)
{
    // "-" means no mapping.
    if (token[0] == '-' && token[1] == '\0') {
        out[0] = '\0';
        return 0;
    }

    // Escape sequences: \n, \t, etc.
    if (resolve_escape(token, out)) return 1;

    // Named special keys: UP, DOWN, F1, etc.
    if (resolve_named_key(token, out)) return 1;

    // Dead keys: DEAD_ACUTE, etc.
    if (resolve_dead_key(token, out)) return 1;

    // Otherwise, treat as a literal UTF-8 string (typically 1 character).
    int len = str_len(token);
    if (len >= KEYMAP_MAX_CHAR_BYTES) len = KEYMAP_MAX_CHAR_BYTES - 1;
    mem_copy(out, token, (size_t)len);
    out[len] = '\0';
    return len;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

int keymap_parse(const char *bkm_data, size_t bkm_length, keymap_t *out)
{
    (void)bkm_length;  // we rely on NUL termination
    mem_zero(out, sizeof(keymap_t));

    const char *cursor = bkm_data;
    char token[64];
    int line_number = 1;
    int error_count = 0;

    while (*cursor) {
        skip_whitespace(&cursor);

        // Skip blank lines and comments.
        if (*cursor == '\n') { cursor++; line_number++; continue; }
        if (*cursor == '#')  { skip_line(&cursor); line_number++; continue; }
        if (*cursor == '\0') break;

        // Parse evdev code.
        if (*cursor < '0' || *cursor > '9') {
            // Unrecognized line — report and skip.
            error_count++;
            skip_line(&cursor);
            line_number++;
            continue;
        }
        int evdev_code = parse_int(&cursor);
        if (evdev_code < 0 || evdev_code >= KEYMAP_MAX_EVDEV_CODE) {
            // Evdev code out of range — report and skip.
            error_count++;
            skip_line(&cursor);
            line_number++;
            continue;
        }

        // Parse up to 4 modifier columns: normal, shift, altgr, shift+altgr.
        int columns_parsed = 0;
        for (int mod = 0; mod < KEYMAP_MODIFIER_COUNT; mod++) {
            int len = parse_token(&cursor, token, sizeof(token));
            if (len == 0) break;  // fewer columns than expected — OK
            resolve_token(token, out->table[evdev_code][mod]);
            columns_parsed++;
        }

        if (columns_parsed == 0)
            error_count++;  // line had evdev code but no mappings

        skip_line(&cursor);
        line_number++;
    }

    return error_count > 0 ? -error_count : 0;
}

const char *keymap_lookup(const keymap_t *keymap, uint16_t evdev_code, int modifier)
{
    if (evdev_code >= KEYMAP_MAX_EVDEV_CODE || modifier < 0 || modifier >= KEYMAP_MODIFIER_COUNT)
        return "";
    return keymap->table[evdev_code][modifier];
}

uint8_t keymap_is_dead_key(const char *mapping)
{
    uint8_t first = (uint8_t)mapping[0];
    if (first >= KEYMAP_DEAD_ACUTE && first <= KEYMAP_DEAD_DIAERESIS)
        return first;
    return 0;
}

// ---------------------------------------------------------------------------
// Dead key composition table
// ---------------------------------------------------------------------------

typedef struct {
    uint8_t  dead_key;
    uint32_t base_codepoint;
    uint32_t composed_codepoint;
} compose_rule_t;

static const compose_rule_t compose_table[] = {
    // DEAD_ACUTE
    { KEYMAP_DEAD_ACUTE, 'a', 0xE1 },  // á
    { KEYMAP_DEAD_ACUTE, 'e', 0xE9 },  // é
    { KEYMAP_DEAD_ACUTE, 'i', 0xED },  // í
    { KEYMAP_DEAD_ACUTE, 'o', 0xF3 },  // ó
    { KEYMAP_DEAD_ACUTE, 'u', 0xFA },  // ú
    { KEYMAP_DEAD_ACUTE, 'y', 0xFD },  // ý
    { KEYMAP_DEAD_ACUTE, 'n', 0x144 }, // ń
    { KEYMAP_DEAD_ACUTE, 'A', 0xC1 },  // Á
    { KEYMAP_DEAD_ACUTE, 'E', 0xC9 },  // É
    { KEYMAP_DEAD_ACUTE, 'I', 0xCD },  // Í
    { KEYMAP_DEAD_ACUTE, 'O', 0xD3 },  // Ó
    { KEYMAP_DEAD_ACUTE, 'U', 0xDA },  // Ú
    { KEYMAP_DEAD_ACUTE, 'Y', 0xDD },  // Ý

    // DEAD_GRAVE
    { KEYMAP_DEAD_GRAVE, 'a', 0xE0 },  // à
    { KEYMAP_DEAD_GRAVE, 'e', 0xE8 },  // è
    { KEYMAP_DEAD_GRAVE, 'i', 0xEC },  // ì
    { KEYMAP_DEAD_GRAVE, 'o', 0xF2 },  // ò
    { KEYMAP_DEAD_GRAVE, 'u', 0xF9 },  // ù
    { KEYMAP_DEAD_GRAVE, 'A', 0xC0 },  // À
    { KEYMAP_DEAD_GRAVE, 'E', 0xC8 },  // È
    { KEYMAP_DEAD_GRAVE, 'I', 0xCC },  // Ì
    { KEYMAP_DEAD_GRAVE, 'O', 0xD2 },  // Ò
    { KEYMAP_DEAD_GRAVE, 'U', 0xD9 },  // Ù

    // DEAD_CIRCUMFLEX
    { KEYMAP_DEAD_CIRCUMFLEX, 'a', 0xE2 },  // â
    { KEYMAP_DEAD_CIRCUMFLEX, 'e', 0xEA },  // ê
    { KEYMAP_DEAD_CIRCUMFLEX, 'i', 0xEE },  // î
    { KEYMAP_DEAD_CIRCUMFLEX, 'o', 0xF4 },  // ô
    { KEYMAP_DEAD_CIRCUMFLEX, 'u', 0xFB },  // û
    { KEYMAP_DEAD_CIRCUMFLEX, 'A', 0xC2 },  // Â
    { KEYMAP_DEAD_CIRCUMFLEX, 'E', 0xCA },  // Ê
    { KEYMAP_DEAD_CIRCUMFLEX, 'I', 0xCE },  // Î
    { KEYMAP_DEAD_CIRCUMFLEX, 'O', 0xD4 },  // Ô
    { KEYMAP_DEAD_CIRCUMFLEX, 'U', 0xDB },  // Û

    // DEAD_TILDE
    { KEYMAP_DEAD_TILDE, 'a', 0xE3 },  // ã
    { KEYMAP_DEAD_TILDE, 'n', 0xF1 },  // ñ
    { KEYMAP_DEAD_TILDE, 'o', 0xF5 },  // õ
    { KEYMAP_DEAD_TILDE, 'A', 0xC3 },  // Ã
    { KEYMAP_DEAD_TILDE, 'N', 0xD1 },  // Ñ
    { KEYMAP_DEAD_TILDE, 'O', 0xD5 },  // Õ

    // DEAD_DIAERESIS
    { KEYMAP_DEAD_DIAERESIS, 'a', 0xE4 },  // ä
    { KEYMAP_DEAD_DIAERESIS, 'e', 0xEB },  // ë
    { KEYMAP_DEAD_DIAERESIS, 'i', 0xEF },  // ï
    { KEYMAP_DEAD_DIAERESIS, 'o', 0xF6 },  // ö
    { KEYMAP_DEAD_DIAERESIS, 'u', 0xFC },  // ü
    { KEYMAP_DEAD_DIAERESIS, 'y', 0xFF },  // ÿ
    { KEYMAP_DEAD_DIAERESIS, 'A', 0xC4 },  // Ä
    { KEYMAP_DEAD_DIAERESIS, 'E', 0xCB },  // Ë
    { KEYMAP_DEAD_DIAERESIS, 'I', 0xCF },  // Ï
    { KEYMAP_DEAD_DIAERESIS, 'O', 0xD6 },  // Ö
    { KEYMAP_DEAD_DIAERESIS, 'U', 0xDC },  // Ü
};

#define COMPOSE_TABLE_SIZE (sizeof(compose_table) / sizeof(compose_table[0]))

int keymap_dead_key_literal(uint8_t dead_key, char *out)
{
    switch (dead_key) {
        case KEYMAP_DEAD_ACUTE:      out[0] = '\''; out[1] = '\0'; return 1;
        case KEYMAP_DEAD_GRAVE:      out[0] = '`';  out[1] = '\0'; return 1;
        case KEYMAP_DEAD_CIRCUMFLEX: out[0] = '^';  out[1] = '\0'; return 1;
        case KEYMAP_DEAD_TILDE:      out[0] = '~';  out[1] = '\0'; return 1;
        case KEYMAP_DEAD_DIAERESIS:  out[0] = '"';  out[1] = '\0'; return 1;
        default:                     out[0] = '\0';                 return 0;
    }
}

int keymap_compose_dead_key(uint8_t dead_key, const char *base, char *out)
{
    // Decode the base character to a codepoint.
    const char *tmp = base;
    uint32_t base_cp = utf8_decode(&tmp);

    for (size_t i = 0; i < COMPOSE_TABLE_SIZE; i++) {
        if (compose_table[i].dead_key == dead_key &&
            compose_table[i].base_codepoint == base_cp) {
            return utf8_encode(compose_table[i].composed_codepoint, out);
        }
    }

    // No compose rule found.
    out[0] = '\0';
    return 0;
}
