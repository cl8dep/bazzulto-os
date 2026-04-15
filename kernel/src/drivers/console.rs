// Framebuffer console — 8×8 bitmap font, UTF-8 aware
//
// Mirrors the C console.c design: direct writes to the Limine framebuffer,
// pixel-scroll on overflow, cursor tracked in character columns/rows.
//
// Color scheme matches the C kernel (Monokai-inspired):
//   FG = 0x00F8F8F2 (off-white)
//   BG = 0x00272822 (dark grey)
//
// ANSI escape sequences (CSI subset):
//   ESC [ <params> J  — erase display (param 2: full clear)
//   ESC [ <params> H  — cursor position
//   ESC [ <params> A  — cursor up
//   ESC [ <params> B  — cursor down
//   ESC [ <params> C  — cursor right
//   ESC [ <params> D  — cursor left
//   ESC [ <params> K  — erase to end of line (param 0 or absent)
//   ESC [ <params> m  — SGR: color / attribute

use core::ptr::{read_volatile, write_volatile};

const CHAR_WIDTH: usize = 8;
const CHAR_HEIGHT: usize = 8;
const COLOR_FG: u32 = 0x00F8F8F2;
const COLOR_BG: u32 = 0x00272822;

// ---------------------------------------------------------------------------
// Monokai palette — standard 8 foreground colors (indices 0–7)
// ---------------------------------------------------------------------------

/// ANSI foreground color table (codes 30–37 → indices 0–7).
/// Monokai-inspired values matching the C kernel color scheme.
const ANSI_FOREGROUND_COLORS: [u32; 8] = [
    0x00555555, // 30 — dark gray
    0x00F92672, // 31 — red
    0x00A6E22E, // 32 — green
    0x00E6DB74, // 33 — yellow
    0x0066D9EF, // 34 — blue (cyan-blue, Monokai)
    0x00AE81FF, // 35 — magenta
    0x00A1EFE4, // 36 — cyan
    0x00F8F8F2, // 37 — white (default FG)
];

/// ANSI background color table (codes 40–47 → indices 0–7).
const ANSI_BACKGROUND_COLORS: [u32; 8] = [
    0x00272822, // 40 — dark (default BG)
    0x00F92672, // 41 — red
    0x00A6E22E, // 42 — green
    0x00E6DB74, // 43 — yellow
    0x0066D9EF, // 44 — blue
    0x00AE81FF, // 45 — magenta
    0x00A1EFE4, // 46 — cyan
    0x00F8F8F2, // 47 — white
];

/// Bright foreground colors (codes 90–97 → indices 0–7).
const ANSI_BRIGHT_FOREGROUND_COLORS: [u32; 8] = [
    0x00888888, // 90 — bright dark gray
    0x00FF6188, // 91 — bright red
    0x00CCF742, // 92 — bright green
    0x00FFE566, // 93 — bright yellow
    0x0099DDFF, // 94 — bright blue
    0x00CC99FF, // 95 — bright magenta
    0x00C8FAFF, // 96 — bright cyan
    0x00FFFFFF, // 97 — bright white
];

// ---------------------------------------------------------------------------
// ANSI parser state
// ---------------------------------------------------------------------------

/// State machine for parsing ANSI/VT100 escape sequences.
#[derive(Clone, Copy)]
enum AnsiState {
    /// No active escape sequence.
    Normal,
    /// Received ESC (0x1B); waiting for the next byte.
    Escape,
    /// Inside CSI sequence (ESC [); accumulating parameters.
    Csi,
}

impl AnsiState {
    const fn new() -> Self {
        AnsiState::Normal
    }
}

/// Maximum number of CSI parameters we track.
const ANSI_MAX_PARAMS: usize = 8;

// ---------------------------------------------------------------------------
// Console state (single global; kernel is single-core at this stage)
// ---------------------------------------------------------------------------

struct ConsoleState {
    address: *mut u32,
    width: usize,
    pitch_pixels: usize, // pitch in 32-bit pixel units
    columns: usize,
    rows: usize,
    cursor_x: usize,
    cursor_y: usize,
    // Active foreground / background colors (may change via SGR).
    current_fg: u32,
    current_bg: u32,
    // ANSI escape parser.
    ansi_state: AnsiState,
    ansi_params: [u8; ANSI_MAX_PARAMS],
    ansi_param_count: usize,
    ansi_current_param: u8,
}

static mut CONSOLE: ConsoleState = ConsoleState {
    address: core::ptr::null_mut(),
    width: 0,
    pitch_pixels: 0,
    columns: 0,
    rows: 0,
    cursor_x: 0,
    cursor_y: 0,
    current_fg: COLOR_FG,
    current_bg: COLOR_BG,
    ansi_state: AnsiState::Normal,
    ansi_params: [0u8; ANSI_MAX_PARAMS],
    ansi_param_count: 0,
    ansi_current_param: 0,
};

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Initialise the console from a Limine framebuffer pointer.
///
/// # Safety
/// Must be called once before any print call, from a single thread.
pub unsafe fn init(address: *mut u32, width: u64, height: u64, pitch_bytes: u64) {
    let pitch_pixels = (pitch_bytes / 4) as usize;
    let columns = width as usize / CHAR_WIDTH;
    let rows = height as usize / CHAR_HEIGHT;

    CONSOLE = ConsoleState {
        address,
        width: width as usize,
        pitch_pixels,
        columns,
        rows,
        cursor_x: 0,
        cursor_y: 0,
        current_fg: COLOR_FG,
        current_bg: COLOR_BG,
        ansi_state: AnsiState::new(),
        ansi_params: [0u8; ANSI_MAX_PARAMS],
        ansi_param_count: 0,
        ansi_current_param: 0,
    };

    // Fill framebuffer with black — userspace (bzdisplayd) owns the display
    // from the first frame.  Kernel output goes to UART only.
    let pixel_count = (height as usize) * pitch_pixels;
    for i in 0..pixel_count {
        write_volatile(address.add(i), 0x00000000u32);
    }

    // Tell the TTY driver about our dimensions.
    crate::drivers::tty::tty_set_winsize(rows as u16, columns as u16);
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

unsafe fn draw_glyph(codepoint: u32, col: usize, row: usize) {
    let glyph = font_lookup(codepoint);
    let pixel_x = col * CHAR_WIDTH;
    let pixel_y = row * CHAR_HEIGHT;
    let c = &*(&raw const CONSOLE);

    for y in 0..CHAR_HEIGHT {
        for x in 0..CHAR_WIDTH {
            let color = if glyph[y] & (1 << x) != 0 { c.current_fg } else { c.current_bg };
            write_volatile(
                c.address.add((pixel_y + y) * c.pitch_pixels + (pixel_x + x)),
                color,
            );
        }
    }
}

/// Fill a rectangular region of pixels with `color`.
unsafe fn fill_pixels(pixel_x: usize, pixel_y: usize, pixel_w: usize, pixel_h: usize, color: u32) {
    let c = &*(&raw const CONSOLE);
    for y in pixel_y..pixel_y + pixel_h {
        for x in pixel_x..pixel_x + pixel_w {
            write_volatile(c.address.add(y * c.pitch_pixels + x), color);
        }
    }
}

unsafe fn scroll_up() {
    let c = &*(&raw const CONSOLE);
    // Copy every row up by one character height.
    for y in 0..(c.rows - 1) * CHAR_HEIGHT {
        for x in 0..c.width {
            let src = read_volatile(c.address.add((y + CHAR_HEIGHT) * c.pitch_pixels + x));
            write_volatile(c.address.add(y * c.pitch_pixels + x), src);
        }
    }
    // Clear the last row.
    let bg = c.current_bg;
    for y in (c.rows - 1) * CHAR_HEIGHT..c.rows * CHAR_HEIGHT {
        for x in 0..c.width {
            write_volatile(c.address.add(y * c.pitch_pixels + x), bg);
        }
    }
}

// ---------------------------------------------------------------------------
// ANSI SGR (Select Graphic Rendition) handler
// ---------------------------------------------------------------------------

/// Apply a single SGR parameter value.
unsafe fn apply_sgr_param(con: &mut ConsoleState, param: u8) {
    match param {
        0 => {
            // Reset to defaults.
            con.current_fg = COLOR_FG;
            con.current_bg = COLOR_BG;
        }
        1 => {
            // Bold — ignored (no bold glyph variants available).
        }
        30..=37 => {
            con.current_fg = ANSI_FOREGROUND_COLORS[(param - 30) as usize];
        }
        40..=47 => {
            con.current_bg = ANSI_BACKGROUND_COLORS[(param - 40) as usize];
        }
        90..=97 => {
            con.current_fg = ANSI_BRIGHT_FOREGROUND_COLORS[(param - 90) as usize];
        }
        _ => {} // unrecognised code — ignore
    }
}

// ---------------------------------------------------------------------------
// ANSI CSI command executor
// ---------------------------------------------------------------------------

/// Execute a complete CSI sequence whose final byte is `command`.
///
/// `params` holds the numeric parameters already parsed (may be fewer than
/// `param_count` if the accumulation overflowed, but we cap at ANSI_MAX_PARAMS).
unsafe fn execute_csi_command(con: &mut ConsoleState, command: u8) {
    // Helper: fetch parameter at index with a default value.
    let param = |index: usize, default: usize| -> usize {
        if index < con.ansi_param_count {
            con.ansi_params[index] as usize
        } else {
            default
        }
    };

    match command {
        b'J' => {
            // Erase display.
            if param(0, 0) == 2 {
                // Clear entire screen.
                let bg = con.current_bg;
                let total = con.rows * CHAR_HEIGHT * con.pitch_pixels;
                for i in 0..total {
                    write_volatile(con.address.add(i), bg);
                }
                con.cursor_x = 0;
                con.cursor_y = 0;
            }
        }
        b'H' => {
            // Cursor position: row (1-based), col (1-based).
            let row = param(0, 1).saturating_sub(1);
            let col = param(1, 1).saturating_sub(1);
            con.cursor_y = row.min(con.rows.saturating_sub(1));
            con.cursor_x = col.min(con.columns.saturating_sub(1));
        }
        b'A' => {
            // Cursor up.
            let distance = param(0, 1);
            con.cursor_y = con.cursor_y.saturating_sub(distance);
        }
        b'B' => {
            // Cursor down.
            let distance = param(0, 1);
            con.cursor_y = (con.cursor_y + distance).min(con.rows.saturating_sub(1));
        }
        b'C' => {
            // Cursor right.
            let distance = param(0, 1);
            con.cursor_x = (con.cursor_x + distance).min(con.columns.saturating_sub(1));
        }
        b'D' => {
            // Cursor left.
            let distance = param(0, 1);
            con.cursor_x = con.cursor_x.saturating_sub(distance);
        }
        b'K' => {
            // Erase to end of line (param 0 or absent).
            if param(0, 0) == 0 {
                let start_pixel_x = con.cursor_x * CHAR_WIDTH;
                let end_pixel_x = con.columns * CHAR_WIDTH;
                let pixel_y = con.cursor_y * CHAR_HEIGHT;
                let bg = con.current_bg;
                fill_pixels(start_pixel_x, pixel_y, end_pixel_x - start_pixel_x, CHAR_HEIGHT, bg);
            }
        }
        b'm' => {
            // SGR — apply all accumulated parameters.
            if con.ansi_param_count == 0 {
                // ESC [ m with no params = reset.
                apply_sgr_param(con, 0);
            } else {
                for index in 0..con.ansi_param_count {
                    let value = con.ansi_params[index];
                    apply_sgr_param(con, value);
                }
            }
        }
        _ => {} // unrecognised final byte — ignore
    }
}

// ---------------------------------------------------------------------------
// Character output
// ---------------------------------------------------------------------------

unsafe fn put_char_at(codepoint: u32, col: usize, row: usize) {
    draw_glyph(codepoint, col, row);
}

/// Print a string.
///
/// During normal boot all kernel output is sent to UART only — the framebuffer
/// is left blank so that userspace (bzinit / bzdisplayd) can take full control
/// of the display from the first frame.
///
/// The framebuffer console machinery (`print_char`, `scroll_up`, etc.) is
/// retained for the kernel panic screen, which must remain visible even when
/// the display server is not running.
pub fn print_str(s: &str) {
    crate::drivers::uart::puts(s);
}

pub unsafe fn print_char(c: char) {
    let con = &mut *(&raw mut CONSOLE);

    // ------------------------------------------------------------------
    // ANSI escape state machine
    // ------------------------------------------------------------------
    match con.ansi_state {
        AnsiState::Escape => {
            if c == '[' {
                // Begin CSI sequence.
                con.ansi_state = AnsiState::Csi;
                con.ansi_param_count = 0;
                con.ansi_current_param = 0;
                for slot in con.ansi_params.iter_mut() {
                    *slot = 0;
                }
            } else {
                // Not a CSI introducer — discard and return to Normal.
                con.ansi_state = AnsiState::Normal;
            }
            return;
        }
        AnsiState::Csi => {
            let byte = c as u8;
            if byte >= b'0' && byte <= b'9' {
                // Accumulate decimal digit (saturate at 255 to fit u8 param).
                let digit = byte - b'0';
                con.ansi_current_param =
                    con.ansi_current_param.saturating_mul(10).saturating_add(digit);
            } else if byte == b';' {
                // Parameter separator — push current and reset.
                if con.ansi_param_count < ANSI_MAX_PARAMS {
                    con.ansi_params[con.ansi_param_count] = con.ansi_current_param;
                    con.ansi_param_count += 1;
                }
                con.ansi_current_param = 0;
            } else if (byte >= b'A' && byte <= b'Z') || (byte >= b'a' && byte <= b'z') {
                // Final byte — push last param and execute.
                if con.ansi_param_count < ANSI_MAX_PARAMS {
                    con.ansi_params[con.ansi_param_count] = con.ansi_current_param;
                    con.ansi_param_count += 1;
                }
                execute_csi_command(con, byte);
                con.ansi_state = AnsiState::Normal;
            } else {
                // Invalid byte inside CSI — abandon sequence.
                con.ansi_state = AnsiState::Normal;
            }
            return;
        }
        AnsiState::Normal => {
            // Check for ESC to start a new sequence.
            if c == '\x1b' {
                con.ansi_state = AnsiState::Escape;
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Normal character output
    // ------------------------------------------------------------------
    match c {
        '\n' => {
            con.cursor_x = 0;
            con.cursor_y += 1;
        }
        '\r' => {
            con.cursor_x = 0;
        }
        _ => {
            put_char_at(c as u32, con.cursor_x, con.cursor_y);
            con.cursor_x += 1;
            if con.cursor_x >= con.columns {
                con.cursor_x = 0;
                con.cursor_y += 1;
            }
        }
    }

    if con.cursor_y >= con.rows {
        scroll_up();
        con.cursor_y = con.rows - 1;
    }
}

pub fn println(s: &str) {
    print_str(s);
    print_str("\n");
}

/// Print an unsigned 64-bit integer in decimal on the framebuffer console.
pub fn print_dec(value: u64) {
    if value == 0 {
        print_str("0");
        return;
    }
    let mut digits = [0u8; 20];
    let mut count = 0usize;
    let mut remaining = value;
    while remaining > 0 {
        digits[count] = b'0' + (remaining % 10) as u8;
        count += 1;
        remaining /= 10;
    }
    // Emit most-significant digit first using print_str on a 1-byte slice.
    for i in (0..count).rev() {
        // SAFETY: digits[i] is always an ASCII digit b'0'..=b'9', valid UTF-8.
        let digit_str = unsafe { core::str::from_utf8_unchecked(&digits[i..=i]) };
        print_str(digit_str);
    }
}

// ---------------------------------------------------------------------------
// Kernel panic screen — guaranteed framebuffer output with no heap/locks
// ---------------------------------------------------------------------------

/// Render a full-screen kernel panic banner on the framebuffer AND serial.
///
/// Clears the display with a red background, prints a prominent "KERNEL PANIC"
/// header, then prints `header` as the first line of context.
///
/// Safe to call from any context: panic handlers, exception handlers, early boot.
/// Writes directly to the framebuffer pixel buffer without any heap allocation
/// or lock, so it works even after the allocator or scheduler have been
/// corrupted.
pub fn console_panic_screen(header: &str) {
    // Also send to serial for logging.
    print_str("\x1b[41m\x1b[97m\x1b[2J\x1b[H");
    print_str("================================================================\n");
    print_str("                      !! KERNEL PANIC !!                       \n");
    print_str("================================================================\n\n");
    print_str(header);
    print_str("\n");

    // Render directly on the framebuffer.
    unsafe {
        let c = &*(&raw const CONSOLE);
        if c.address.is_null() || c.width == 0 {
            return; // Framebuffer not initialized yet.
        }

        const PANIC_RED: u32 = 0x00CC0000;
        const PANIC_WHITE: u32 = 0x00FFFFFF;
        const PANIC_DARK_RED: u32 = 0x00880000;

        // Fill entire screen with red.
        let total_pixels = c.rows * CHAR_HEIGHT * c.pitch_pixels;
        for i in 0..total_pixels {
            write_volatile(c.address.add(i), PANIC_RED);
        }

        // Save and set panic colors.
        let saved_fg = c.current_fg;
        let saved_bg = c.current_bg;
        let c_mut = &mut *(&raw mut CONSOLE);
        c_mut.current_fg = PANIC_WHITE;
        c_mut.current_bg = PANIC_DARK_RED;

        // Draw banner bar (3 rows of dark red background).
        let banner_y = 2; // Start at row 2
        fill_pixels(0, banner_y * CHAR_HEIGHT, c.width, 3 * CHAR_HEIGHT, PANIC_DARK_RED);

        // Draw "!! KERNEL PANIC !!" centered on the banner.
        let title = "!! KERNEL PANIC !!";
        let title_col = (c.columns.saturating_sub(title.len())) / 2;
        for (i, ch) in title.chars().enumerate() {
            draw_glyph(ch as u32, title_col + i, banner_y + 1);
        }

        // Draw the message body below the banner.
        c_mut.current_fg = PANIC_WHITE;
        c_mut.current_bg = PANIC_RED;
        let mut col = 2;
        let mut row = banner_y + 5;
        for ch in header.chars() {
            if ch == '\n' {
                col = 2;
                row += 1;
                if row >= c.rows { break; }
                continue;
            }
            if col >= c.columns - 2 {
                col = 2;
                row += 1;
                if row >= c.rows { break; }
            }
            draw_glyph(ch as u32, col, row);
            col += 1;
        }

        // Restore colors (not strictly needed since we loop forever, but clean).
        c_mut.current_fg = saved_fg;
        c_mut.current_bg = saved_bg;
    }
}

// ---------------------------------------------------------------------------
// fmt::Write — lets us use write!/writeln! macros
// ---------------------------------------------------------------------------

pub struct ConsoleWriter;

impl core::fmt::Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        print_str(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::drivers::console::ConsoleWriter, $($arg)*);
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = writeln!($crate::drivers::console::ConsoleWriter, $($arg)*);
    }};
}

// ---------------------------------------------------------------------------
// Minimal 8×8 bitmap font
//
// ASCII glyphs (U+0020–U+007F), 8 bytes each, LSB = leftmost pixel.
// Source: public domain PC BIOS font (IBM CP437 subset).
// ---------------------------------------------------------------------------

fn font_lookup(codepoint: u32) -> &'static [u8; 8] {
    if codepoint >= 0x20 && codepoint <= 0x7F {
        let index = (codepoint - 0x20) as usize;
        &FONT_ASCII[index]
    } else {
        &FONT_FALLBACK
    }
}

static FONT_FALLBACK: [u8; 8] = [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00]; // '?'

#[rustfmt::skip]
static FONT_ASCII: [[u8; 8]; 96] = [
    // 0x20 ' '
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00],
    // 0x21 '!'
    [0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00],
    // 0x22 '"'
    [0x36,0x36,0x00,0x00,0x00,0x00,0x00,0x00],
    // 0x23 '#'
    [0x36,0x36,0x7F,0x36,0x7F,0x36,0x36,0x00],
    // 0x24 '$'
    [0x0C,0x3E,0x03,0x1E,0x30,0x1F,0x0C,0x00],
    // 0x25 '%'
    [0x00,0x63,0x33,0x18,0x0C,0x66,0x63,0x00],
    // 0x26 '&'
    [0x1C,0x36,0x1C,0x6E,0x3B,0x33,0x6E,0x00],
    // 0x27 '\''
    [0x06,0x06,0x03,0x00,0x00,0x00,0x00,0x00],
    // 0x28 '('
    [0x18,0x0C,0x06,0x06,0x06,0x0C,0x18,0x00],
    // 0x29 ')'
    [0x06,0x0C,0x18,0x18,0x18,0x0C,0x06,0x00],
    // 0x2A '*'
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00],
    // 0x2B '+'
    [0x00,0x0C,0x0C,0x3F,0x0C,0x0C,0x00,0x00],
    // 0x2C ','
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x06],
    // 0x2D '-'
    [0x00,0x00,0x00,0x3F,0x00,0x00,0x00,0x00],
    // 0x2E '.'
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x00],
    // 0x2F '/'
    [0x60,0x30,0x18,0x0C,0x06,0x03,0x01,0x00],
    // 0x30 '0'
    [0x3E,0x63,0x73,0x7B,0x6F,0x67,0x3E,0x00],
    // 0x31 '1'
    [0x0C,0x0E,0x0C,0x0C,0x0C,0x0C,0x3F,0x00],
    // 0x32 '2'
    [0x1E,0x33,0x30,0x1C,0x06,0x33,0x3F,0x00],
    // 0x33 '3'
    [0x1E,0x33,0x30,0x1C,0x30,0x33,0x1E,0x00],
    // 0x34 '4'
    [0x38,0x3C,0x36,0x33,0x7F,0x30,0x78,0x00],
    // 0x35 '5'
    [0x3F,0x03,0x1F,0x30,0x30,0x33,0x1E,0x00],
    // 0x36 '6'
    [0x1C,0x06,0x03,0x1F,0x33,0x33,0x1E,0x00],
    // 0x37 '7'
    [0x3F,0x33,0x30,0x18,0x0C,0x0C,0x0C,0x00],
    // 0x38 '8'
    [0x1E,0x33,0x33,0x1E,0x33,0x33,0x1E,0x00],
    // 0x39 '9'
    [0x1E,0x33,0x33,0x3E,0x30,0x18,0x0E,0x00],
    // 0x3A ':'
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x00],
    // 0x3B ';'
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x06],
    // 0x3C '<'
    [0x18,0x0C,0x06,0x03,0x06,0x0C,0x18,0x00],
    // 0x3D '='
    [0x00,0x00,0x3F,0x00,0x00,0x3F,0x00,0x00],
    // 0x3E '>'
    [0x06,0x0C,0x18,0x30,0x18,0x0C,0x06,0x00],
    // 0x3F '?'
    [0x1E,0x33,0x30,0x18,0x0C,0x00,0x0C,0x00],
    // 0x40 '@'
    [0x3E,0x63,0x7B,0x7B,0x7B,0x03,0x1E,0x00],
    // 0x41 'A'
    [0x0C,0x1E,0x33,0x33,0x3F,0x33,0x33,0x00],
    // 0x42 'B'
    [0x3F,0x66,0x66,0x3E,0x66,0x66,0x3F,0x00],
    // 0x43 'C'
    [0x3C,0x66,0x03,0x03,0x03,0x66,0x3C,0x00],
    // 0x44 'D'
    [0x1F,0x36,0x66,0x66,0x66,0x36,0x1F,0x00],
    // 0x45 'E'
    [0x7F,0x46,0x16,0x1E,0x16,0x46,0x7F,0x00],
    // 0x46 'F'
    [0x7F,0x46,0x16,0x1E,0x16,0x06,0x0F,0x00],
    // 0x47 'G'
    [0x3C,0x66,0x03,0x03,0x73,0x66,0x7C,0x00],
    // 0x48 'H'
    [0x33,0x33,0x33,0x3F,0x33,0x33,0x33,0x00],
    // 0x49 'I'
    [0x1E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00],
    // 0x4A 'J'
    [0x78,0x30,0x30,0x30,0x33,0x33,0x1E,0x00],
    // 0x4B 'K'
    [0x67,0x66,0x36,0x1E,0x36,0x66,0x67,0x00],
    // 0x4C 'L'
    [0x0F,0x06,0x06,0x06,0x46,0x66,0x7F,0x00],
    // 0x4D 'M'
    [0x63,0x77,0x7F,0x7F,0x6B,0x63,0x63,0x00],
    // 0x4E 'N'
    [0x63,0x67,0x6F,0x7B,0x73,0x63,0x63,0x00],
    // 0x4F 'O'
    [0x1C,0x36,0x63,0x63,0x63,0x36,0x1C,0x00],
    // 0x50 'P'
    [0x3F,0x66,0x66,0x3E,0x06,0x06,0x0F,0x00],
    // 0x51 'Q'
    [0x1E,0x33,0x33,0x33,0x3B,0x1E,0x38,0x00],
    // 0x52 'R'
    [0x3F,0x66,0x66,0x3E,0x36,0x66,0x67,0x00],
    // 0x53 'S'
    [0x1E,0x33,0x07,0x0E,0x38,0x33,0x1E,0x00],
    // 0x54 'T'
    [0x3F,0x2D,0x0C,0x0C,0x0C,0x0C,0x1E,0x00],
    // 0x55 'U'
    [0x33,0x33,0x33,0x33,0x33,0x33,0x3F,0x00],
    // 0x56 'V'
    [0x33,0x33,0x33,0x33,0x33,0x1E,0x0C,0x00],
    // 0x57 'W'
    [0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00],
    // 0x58 'X'
    [0x63,0x63,0x36,0x1C,0x1C,0x36,0x63,0x00],
    // 0x59 'Y'
    [0x33,0x33,0x33,0x1E,0x0C,0x0C,0x1E,0x00],
    // 0x5A 'Z'
    [0x7F,0x63,0x31,0x18,0x4C,0x66,0x7F,0x00],
    // 0x5B '['
    [0x1E,0x06,0x06,0x06,0x06,0x06,0x1E,0x00],
    // 0x5C '\'
    [0x03,0x06,0x0C,0x18,0x30,0x60,0x40,0x00],
    // 0x5D ']'
    [0x1E,0x18,0x18,0x18,0x18,0x18,0x1E,0x00],
    // 0x5E '^'
    [0x08,0x1C,0x36,0x63,0x00,0x00,0x00,0x00],
    // 0x5F '_'
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF],
    // 0x60 '`'
    [0x0C,0x0C,0x18,0x00,0x00,0x00,0x00,0x00],
    // 0x61 'a'
    [0x00,0x00,0x1E,0x30,0x3E,0x33,0x6E,0x00],
    // 0x62 'b'
    [0x07,0x06,0x06,0x3E,0x66,0x66,0x3B,0x00],
    // 0x63 'c'
    [0x00,0x00,0x1E,0x33,0x03,0x33,0x1E,0x00],
    // 0x64 'd'
    [0x38,0x30,0x30,0x3E,0x33,0x33,0x6E,0x00],
    // 0x65 'e'
    [0x00,0x00,0x1E,0x33,0x3F,0x03,0x1E,0x00],
    // 0x66 'f'
    [0x1C,0x36,0x06,0x0F,0x06,0x06,0x0F,0x00],
    // 0x67 'g'
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x1F],
    // 0x68 'h'
    [0x07,0x06,0x36,0x6E,0x66,0x66,0x67,0x00],
    // 0x69 'i'
    [0x0C,0x00,0x0E,0x0C,0x0C,0x0C,0x1E,0x00],
    // 0x6A 'j'
    [0x30,0x00,0x30,0x30,0x30,0x33,0x33,0x1E],
    // 0x6B 'k'
    [0x07,0x06,0x66,0x36,0x1E,0x36,0x67,0x00],
    // 0x6C 'l'
    [0x0E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00],
    // 0x6D 'm'
    [0x00,0x00,0x33,0x7F,0x7F,0x6B,0x63,0x00],
    // 0x6E 'n'
    [0x00,0x00,0x1F,0x33,0x33,0x33,0x33,0x00],
    // 0x6F 'o'
    [0x00,0x00,0x1E,0x33,0x33,0x33,0x1E,0x00],
    // 0x70 'p'
    [0x00,0x00,0x3B,0x66,0x66,0x3E,0x06,0x0F],
    // 0x71 'q'
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x78],
    // 0x72 'r'
    [0x00,0x00,0x3B,0x6E,0x66,0x06,0x0F,0x00],
    // 0x73 's'
    [0x00,0x00,0x3E,0x03,0x1E,0x30,0x1F,0x00],
    // 0x74 't'
    [0x08,0x0C,0x3E,0x0C,0x0C,0x2C,0x18,0x00],
    // 0x75 'u'
    [0x00,0x00,0x33,0x33,0x33,0x33,0x6E,0x00],
    // 0x76 'v'
    [0x00,0x00,0x33,0x33,0x33,0x1E,0x0C,0x00],
    // 0x77 'w'
    [0x00,0x00,0x63,0x6B,0x7F,0x7F,0x36,0x00],
    // 0x78 'x'
    [0x00,0x00,0x63,0x36,0x1C,0x36,0x63,0x00],
    // 0x79 'y'
    [0x00,0x00,0x33,0x33,0x33,0x3E,0x30,0x1F],
    // 0x7A 'z'
    [0x00,0x00,0x3F,0x19,0x0C,0x26,0x3F,0x00],
    // 0x7B '{'
    [0x38,0x0C,0x0C,0x07,0x0C,0x0C,0x38,0x00],
    // 0x7C '|'
    [0x18,0x18,0x18,0x00,0x18,0x18,0x18,0x00],
    // 0x7D '}'
    [0x07,0x0C,0x0C,0x38,0x0C,0x0C,0x07,0x00],
    // 0x7E '~'
    [0x6E,0x3B,0x00,0x00,0x00,0x00,0x00,0x00],
    // 0x7F DEL (unused, show as block)
    [0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF],
];
