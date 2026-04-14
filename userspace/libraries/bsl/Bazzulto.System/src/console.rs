//! Terminal I/O — Console facade with ANSI color and cursor support.

use crate::raw;
use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

// ---------------------------------------------------------------------------
// KeyInfo
// ---------------------------------------------------------------------------

pub struct KeyInfo {
    pub ch:       Option<char>,
    pub is_ctrl:  bool,
    pub raw_byte: u8,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a byte slice directly to `fd`.
fn write_fd(fd: i32, data: &[u8]) {
    raw::raw_write(fd, data.as_ptr(), data.len());
}

/// Write a byte slice to fd 1.
fn write_ansi(seq: &[u8]) {
    write_fd(1, seq);
}

/// Format a u16 into a stack buffer. Returns the slice containing the digits.
fn u16_to_buf(value: u16, buf: &mut [u8; 5]) -> &[u8] {
    if value == 0 {
        buf[4] = b'0';
        return &buf[4..];
    }
    let mut remaining = value;
    let mut position = 5usize;
    while remaining > 0 {
        position -= 1;
        buf[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    &buf[position..]
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

pub struct Console;

impl Console {
    /// Write text to stdout (fd 1).
    pub fn write(text: &str) {
        write_fd(1, text.as_bytes());
    }

    /// Write text followed by newline to stdout.
    pub fn writeln(text: &str) {
        write_fd(1, text.as_bytes());
        write_fd(1, b"\n");
    }

    /// Write text to stderr (fd 2).
    pub fn write_err(text: &str) {
        write_fd(2, text.as_bytes());
    }

    /// Write text followed by newline to stderr.
    pub fn writeln_err(text: &str) {
        write_fd(2, text.as_bytes());
        write_fd(2, b"\n");
    }

    /// Read a line from stdin (fd 0) until '\n'. Allocates.
    pub fn read_line() -> String {
        let mut result = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let count = raw::raw_read(0, byte.as_mut_ptr(), 1);
            if count <= 0 {
                break;
            }
            if byte[0] == b'\n' {
                break;
            }
            result.push(byte[0]);
        }
        String::from_utf8(result).unwrap_or_default()
    }

    /// Read a single character from stdin (blocking).
    pub fn read_char() -> Option<char> {
        let mut byte = [0u8; 1];
        let count = raw::raw_read(0, byte.as_mut_ptr(), 1);
        if count <= 0 {
            return None;
        }
        // Only handles ASCII for now.
        if byte[0].is_ascii() {
            Some(byte[0] as char)
        } else {
            None
        }
    }

    /// Deferred — requires ioctl for raw mode (see docs/tech-debt/bzinit-v1.md).
    pub fn read_key() -> Option<KeyInfo> {
        None
    }

    /// Set ANSI foreground color.
    pub fn set_foreground(color: Color) {
        let code: u16 = match color {
            Color::Black         => 30,
            Color::Red           => 31,
            Color::Green         => 32,
            Color::Yellow        => 33,
            Color::Blue          => 34,
            Color::Magenta       => 35,
            Color::Cyan          => 36,
            Color::White         => 37,
            Color::BrightBlack   => 90,
            Color::BrightRed     => 91,
            Color::BrightGreen   => 92,
            Color::BrightYellow  => 93,
            Color::BrightBlue    => 94,
            Color::BrightMagenta => 95,
            Color::BrightCyan    => 96,
            Color::BrightWhite   => 97,
        };
        write_ansi(b"\x1b[");
        let mut buf = [0u8; 5];
        write_ansi(u16_to_buf(code, &mut buf));
        write_ansi(b"m");
    }

    /// Set ANSI background color.
    pub fn set_background(color: Color) {
        let code: u16 = match color {
            Color::Black         => 40,
            Color::Red           => 41,
            Color::Green         => 42,
            Color::Yellow        => 43,
            Color::Blue          => 44,
            Color::Magenta       => 45,
            Color::Cyan          => 46,
            Color::White         => 47,
            Color::BrightBlack   => 100,
            Color::BrightRed     => 101,
            Color::BrightGreen   => 102,
            Color::BrightYellow  => 103,
            Color::BrightBlue    => 104,
            Color::BrightMagenta => 105,
            Color::BrightCyan    => 106,
            Color::BrightWhite   => 107,
        };
        write_ansi(b"\x1b[");
        let mut buf = [0u8; 5];
        write_ansi(u16_to_buf(code, &mut buf));
        write_ansi(b"m");
    }

    /// Reset all ANSI attributes.
    pub fn reset_color() {
        write_ansi(b"\x1b[0m");
    }

    /// Clear screen and move cursor to home position.
    pub fn clear() {
        write_ansi(b"\x1b[2J\x1b[H");
    }

    /// Move cursor to column `x`, row `y` (1-based).
    pub fn set_cursor(x: u16, y: u16) {
        write_ansi(b"\x1b[");
        let mut buf_y = [0u8; 5];
        write_ansi(u16_to_buf(y, &mut buf_y));
        write_ansi(b";");
        let mut buf_x = [0u8; 5];
        write_ansi(u16_to_buf(x, &mut buf_x));
        write_ansi(b"H");
    }

    /// Hide the terminal cursor.
    pub fn hide_cursor() {
        write_ansi(b"\x1b[?25l");
    }

    /// Show the terminal cursor.
    pub fn show_cursor() {
        write_ansi(b"\x1b[?25h");
    }

    /// Move cursor to column 0 of the current line.
    pub fn carriage_return() {
        write_ansi(b"\r");
    }

    /// Deferred — requires ioctl TIOCGWINSZ (see docs/tech-debt/bzinit-v1.md).
    pub fn width() -> Option<u16> {
        None
    }

    /// Deferred — requires ioctl TIOCGWINSZ (see docs/tech-debt/bzinit-v1.md).
    pub fn height() -> Option<u16> {
        None
    }

    /// Deferred — requires ioctl isatty check (see docs/tech-debt/bzinit-v1.md).
    pub fn is_tty() -> bool {
        false
    }

    /// Deferred — requires ANSI cursor position query (see docs/tech-debt/bzinit-v1.md).
    pub fn cursor_position() -> Option<(u16, u16)> {
        None
    }
}
