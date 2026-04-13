#pragma once
// Bazzulto.System — Console C++ API
//
// Terminal I/O with ANSI color and cursor control.
// All methods are static — Console is a pure facade, not instantiated.

#include <stdint.h>
#include <stddef.h>

namespace Bazzulto {

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

enum class Color : uint8_t {
    Black         = 0,
    Red           = 1,
    Green         = 2,
    Yellow        = 3,
    Blue          = 4,
    Magenta       = 5,
    Cyan          = 6,
    White         = 7,
    BrightBlack   = 8,
    BrightRed     = 9,
    BrightGreen   = 10,
    BrightYellow  = 11,
    BrightBlue    = 12,
    BrightMagenta = 13,
    BrightCyan    = 14,
    BrightWhite   = 15,
};

// ---------------------------------------------------------------------------
// KeyInfo — returned by read_key() (deferred)
// ---------------------------------------------------------------------------

struct KeyInfo {
    /// Unicode code point, or 0 if non-printable.
    uint32_t codepoint;
    bool     is_ctrl;
    uint8_t  raw_byte;
};

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

struct Console {
    Console() = delete;

    // --- Output (fd 1) ---

    static void write(const char* text, size_t length) noexcept;

    static void write(const char* text) noexcept {
        size_t len = 0;
        while (text[len]) ++len;
        write(text, len);
    }

    static void writeln(const char* text) noexcept {
        write(text);
        write("\n", 1);
    }

    // --- Error output (fd 2) ---

    static void write_err(const char* text, size_t length) noexcept;

    static void write_err(const char* text) noexcept {
        size_t len = 0;
        while (text[len]) ++len;
        write_err(text, len);
    }

    static void writeln_err(const char* text) noexcept {
        write_err(text);
        write_err("\n", 1);
    }

    // --- Input (fd 0) ---

    /// Read up to `buf_len - 1` bytes until '\n' or EOF. NUL-terminates buf.
    /// Returns number of bytes read.
    static size_t read_line(char* buf, size_t buf_len) noexcept;

    /// Read one byte. Returns -1 on EOF.
    static int read_char() noexcept;

    /// Read a key without waiting for Enter.
    /// Deferred — requires raw terminal mode (ioctl not yet available).
    /// Always returns false in v1.0.
    static bool read_key(KeyInfo& /*out*/) noexcept { return false; }

    // --- ANSI color ---

    static void set_foreground(Color color) noexcept;
    static void set_background(Color color) noexcept;
    static void reset_color() noexcept;

    // --- ANSI cursor / screen ---

    static void clear() noexcept;
    /// Move cursor to column `x`, row `y` (1-based).
    static void set_cursor(uint16_t x, uint16_t y) noexcept;
    static void hide_cursor() noexcept;
    static void show_cursor() noexcept;
    static void carriage_return() noexcept { write("\r", 1); }

    // --- Terminal info (deferred — needs ioctl TIOCGWINSZ) ---

    /// Returns 0 if unavailable.
    static uint16_t width()  noexcept { return 0; }
    static uint16_t height() noexcept { return 0; }
    static bool     is_tty() noexcept { return false; }
};

} // namespace Bazzulto
