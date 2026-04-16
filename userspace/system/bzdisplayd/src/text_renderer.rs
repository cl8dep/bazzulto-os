//! TextRenderer — rasterizes text onto a FramebufferSurface using FontManager.
//!
//! # Separation of concerns
//!
//!   FontManager   → rasterizes fonts → produces coverage bitmaps
//!   TextRenderer  → places bitmaps on screen, tracks cursor position
//!   FramebufferSurface → writes pixels to the framebuffer
//!
//! The display driver never sees character data — only bitmaps.

use bazzulto_display::font_manager::{FontId, FontManager};
use crate::framebuffer::FramebufferSurface;

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// 24-bit RGB colour.
#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK:   Color = Color { r:   0, g:   0, b:   0 };
    pub const WHITE:   Color = Color { r: 255, g: 255, b: 255 };
    pub const RED:     Color = Color { r: 255, g:   0, b:   0 };
    pub const GREEN:   Color = Color { r:   0, g: 255, b:   0 };
    pub const BLUE:    Color = Color { r:   0, g:   0, b: 255 };
    pub const CYAN:    Color = Color { r:   0, g: 255, b: 255 };
    pub const YELLOW:  Color = Color { r: 255, g: 255, b:   0 };
    pub const GRAY:    Color = Color { r: 128, g: 128, b: 128 };
}

// ---------------------------------------------------------------------------
// TextRenderer
// ---------------------------------------------------------------------------

/// Maximum columns and rows for the screen buffer.
const SCREEN_MAX_COLS: usize = 256;
const SCREEN_MAX_ROWS: usize = 128;

pub struct TextRenderer {
    /// Current pen position — top-left of the next character.
    cursor_x:   u32,
    cursor_y:   u32,
    /// Point size for rasterization.
    point_size: f32,
    foreground: Color,
    background: Color,
    /// Left margin (reset position for carriage return).
    margin_left: u32,
    /// Top margin.
    margin_top:  u32,
    /// Line height in pixels (point_size + leading).
    line_height: u32,
    /// Fixed cell width for monospace rendering.
    ///
    /// Every character occupies this exact width regardless of its individual
    /// glyph metrics.  Computed once at initialization from a reference glyph
    /// ('M') and never changed.  This ensures uniform spacing across the grid.
    cell_width: u32,
    /// Screen buffer: stores the character at each (col, row) cell so
    /// the cursor can be drawn as an inverted block and then restored
    /// without losing the glyph underneath.
    screen: [[char; SCREEN_MAX_COLS]; SCREEN_MAX_ROWS],
}

impl TextRenderer {
    pub fn new(
        point_size:   f32,
        foreground:   Color,
        background:   Color,
        margin_left:  u32,
        margin_top:   u32,
        font_id:      FontId,
        font_manager: &mut FontManager,
    ) -> TextRenderer {
        let line_height = (point_size * 1.25) as u32;
        // Compute fixed cell width from the reference glyph 'M'.
        // For a monospace font every glyph should have the same advance,
        // but fontdue returns floats that truncate differently per glyph.
        // Using a single rounded value eliminates the drift.
        let cell_width = font_manager.rasterize(font_id, 'M', point_size)
            .map(|bm| (bm.advance_width + 0.5) as u32)
            .unwrap_or((point_size * 0.6) as u32);
        TextRenderer {
            cursor_x: margin_left,
            cursor_y: margin_top,
            point_size,
            foreground,
            background,
            margin_left,
            margin_top,
            line_height,
            cell_width,
            screen: [[' '; SCREEN_MAX_COLS]; SCREEN_MAX_ROWS],
        }
    }

    /// Draw a string at the current cursor position.
    ///
    /// Handles `\n` as newline + carriage return.
    /// Wraps at the right edge of the surface.
    pub fn draw_str(
        &mut self,
        text:         &str,
        font_id:      FontId,
        font_manager: &mut FontManager,
        surface:      &mut FramebufferSurface,
    ) {
        for character in text.chars() {
            if character == '\n' {
                self.newline(surface);
                continue;
            }
            if character == '\x08' {
                self.backspace(surface);
                continue;
            }
            self.draw_char(character, font_id, font_manager, surface);
        }
    }

    /// Move the cursor back by one character width and erase that cell.
    ///
    /// Called when a BS (0x08) byte is received.  The TTY line discipline
    /// echoes `\x08 \x08` (BS SPACE BS) for each erased character: the first
    /// BS moves the cursor back, the SPACE overwrites with background, the
    /// second BS repositions ready for new input.
    pub fn backspace(&mut self, surface: &mut FramebufferSurface) {
        if self.cursor_x <= self.margin_left {
            return; // already at left margin — nothing to erase
        }
        self.cursor_x = self.cursor_x.saturating_sub(self.cell_width).max(self.margin_left);
        // Erase the cell by filling it with the background colour.
        surface.fill_rect(
            self.cursor_x,
            self.cursor_y,
            self.cell_width,
            self.line_height,
            self.background.r,
            self.background.g,
            self.background.b,
        );
    }

    /// Draw a block cursor at the current position with inverted colours.
    ///
    /// The character under the cursor (from the screen buffer) is re-rendered
    /// with swapped foreground/background, matching the macOS Terminal style.
    /// If the cell is empty (space), a solid foreground block is drawn.
    pub fn draw_cursor_with_font(
        &mut self,
        font_id: FontId,
        font_manager: &mut FontManager,
        surface: &mut FramebufferSurface,
    ) {
        let advance = self.cell_width.max(1);
        // Fill the cell with foreground colour (the "block").
        surface.fill_rect(
            self.cursor_x, self.cursor_y,
            advance, self.line_height,
            self.foreground.r, self.foreground.g, self.foreground.b,
        );
        // Render the character in background colour (inverted) on top.
        let ch = self.char_at_cursor();
        if ch != ' ' {
            self.render_char_at_cursor(ch, font_id, font_manager, surface,
                self.background, self.foreground);
        }
    }

    /// Erase the block cursor by restoring the character with normal colours.
    pub fn erase_cursor_with_font(
        &mut self,
        font_id: FontId,
        font_manager: &mut FontManager,
        surface: &mut FramebufferSurface,
    ) {
        let advance = self.cell_width.max(1);
        // Fill the cell with background colour.
        surface.fill_rect(
            self.cursor_x, self.cursor_y,
            advance, self.line_height,
            self.background.r, self.background.g, self.background.b,
        );
        // Re-render the character normally.
        let ch = self.char_at_cursor();
        if ch != ' ' {
            self.render_char_at_cursor(ch, font_id, font_manager, surface,
                self.foreground, self.background);
        }
    }

    /// Legacy cursor methods (no font) — used for blink toggle when we
    /// don't want to pass font through.  Kept as thin wrappers.
    pub fn draw_cursor(&self, surface: &mut FramebufferSurface) {
        let advance = self.cell_width.max(1);
        surface.fill_rect(
            self.cursor_x, self.cursor_y,
            advance, self.line_height,
            self.foreground.r, self.foreground.g, self.foreground.b,
        );
    }

    pub fn erase_cursor(&self, surface: &mut FramebufferSurface) {
        let advance = self.cell_width.max(1);
        surface.fill_rect(
            self.cursor_x, self.cursor_y,
            advance, self.line_height,
            self.background.r, self.background.g, self.background.b,
        );
    }

    /// Clear the entire screen and reset the cursor to the home position.
    ///
    /// Called when the ANSI escape sequence `ESC[2J` is received.
    pub fn clear_screen(&mut self, surface: &mut FramebufferSurface) {
        surface.fill_rect(0, 0, surface.width, surface.height,
            self.background.r, self.background.g, self.background.b);
        self.cursor_x = self.margin_left;
        self.cursor_y = self.margin_top;
        self.screen = [[' '; SCREEN_MAX_COLS]; SCREEN_MAX_ROWS];
    }

    /// Reset the cursor to the home position (top-left).
    ///
    /// Called when the ANSI escape sequence `ESC[H` is received.
    pub fn cursor_home(&mut self) {
        self.cursor_x = self.margin_left;
        self.cursor_y = self.margin_top;
    }

    /// Erase from the cursor to the end of the current line.
    ///
    /// Called when the ANSI escape sequence `ESC[K` is received.
    pub fn erase_to_end_of_line(&mut self, surface: &mut FramebufferSurface) {
        let remaining_width = surface.width.saturating_sub(self.cursor_x);
        if remaining_width > 0 {
            surface.fill_rect(
                self.cursor_x, self.cursor_y,
                remaining_width, self.line_height,
                self.background.r, self.background.g, self.background.b,
            );
        }
    }

    /// Move the cursor forward (right) by `count` character cells.
    ///
    /// Called when the ANSI escape sequence `ESC[<n>C` is received.
    pub fn cursor_forward(&mut self, count: u32, surface: &FramebufferSurface) {
        let advance = self.cell_width.max(1);
        self.cursor_x = (self.cursor_x + advance * count).min(
            surface.width.saturating_sub(advance),
        );
    }

    /// Move the cursor backward (left) by `count` character cells.
    ///
    /// Called when the ANSI escape sequence `ESC[<n>D` is received.
    pub fn cursor_backward(&mut self, count: u32) {
        let advance = self.cell_width.max(1);
        self.cursor_x = self.cursor_x
            .saturating_sub(advance * count)
            .max(self.margin_left);
    }

    /// Move cursor to a new line, scrolling the framebuffer if at the bottom.
    pub fn newline(&mut self, surface: &mut FramebufferSurface) {
        self.cursor_x   = self.margin_left;
        self.cursor_y  += self.line_height;

        // Scroll when the next line would exceed the framebuffer height.
        if self.cursor_y + self.line_height > surface.height {
            surface.scroll_up(self.line_height);
            // cursor_y stays at the last visible line start after scrolling.
            self.cursor_y = surface.height.saturating_sub(self.line_height);
        }
    }

    // -----------------------------------------------------------------------
    // Screen buffer helpers
    // -----------------------------------------------------------------------

    /// Current column in character grid coordinates.
    fn cursor_col(&self) -> usize {
        let advance = self.cell_width.max(1);
        ((self.cursor_x.saturating_sub(self.margin_left)) / advance) as usize
    }

    /// Current row in character grid coordinates.
    fn cursor_row(&self) -> usize {
        ((self.cursor_y.saturating_sub(self.margin_top)) / self.line_height) as usize
    }

    /// Get the character stored at the current cursor position.
    fn char_at_cursor(&self) -> char {
        let col = self.cursor_col();
        let row = self.cursor_row();
        if row < SCREEN_MAX_ROWS && col < SCREEN_MAX_COLS {
            self.screen[row][col]
        } else {
            ' '
        }
    }

    /// Store a character in the screen buffer at the current cursor position.
    fn store_char(&mut self, character: char) {
        let col = self.cursor_col();
        let row = self.cursor_row();
        if row < SCREEN_MAX_ROWS && col < SCREEN_MAX_COLS {
            self.screen[row][col] = character;
        }
    }

    /// Render a single character at the current cursor position with
    /// explicit foreground/background colours (used for cursor inversion).
    fn render_char_at_cursor(
        &self,
        character: char,
        font_id: FontId,
        font_manager: &mut FontManager,
        surface: &mut FramebufferSurface,
        fg: Color,
        bg: Color,
    ) {
        let ascender = font_manager.ascender_pixels(font_id, self.point_size);
        if let Some(bitmap) = font_manager.rasterize(font_id, character, self.point_size) {
            let glyph_x = (self.cursor_x as i32 + bitmap.x_offset).max(0) as u32;
            let glyph_top = (self.cursor_y as i32
                + ascender as i32
                - bitmap.y_offset
                - bitmap.height as i32
                + 1)
                .max(0) as u32;
            surface.draw_bitmap(
                glyph_x, glyph_top,
                &bitmap.coverage, bitmap.width, bitmap.height,
                fg.r, fg.g, fg.b,
                bg.r, bg.g, bg.b,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn draw_char(
        &mut self,
        character:    char,
        font_id:      FontId,
        font_manager: &mut FontManager,
        surface:      &mut FramebufferSurface,
    ) {
        // Store in screen buffer before advancing cursor.
        self.store_char(character);

        let ascender = font_manager.ascender_pixels(font_id, self.point_size);
        let bitmap_opt = font_manager.rasterize(font_id, character, self.point_size);

        if let Some(bitmap) = bitmap_opt {
            let glyph_x = (self.cursor_x as i32 + bitmap.x_offset).max(0) as u32;
            let glyph_top = (self.cursor_y as i32
                + ascender as i32
                - bitmap.y_offset
                - bitmap.height as i32
                + 1)
                .max(0) as u32;

            surface.draw_bitmap(
                glyph_x,
                glyph_top,
                &bitmap.coverage,
                bitmap.width,
                bitmap.height,
                self.foreground.r, self.foreground.g, self.foreground.b,
                self.background.r, self.background.g, self.background.b,
            );
        }

        self.cursor_x += self.cell_width;

        // Wrap at right edge.
        if self.cursor_x + self.cell_width > surface.width {
            self.newline(surface);
        }
    }
}
