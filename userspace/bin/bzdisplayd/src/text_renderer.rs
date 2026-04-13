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
    /// Advance width of the last rendered character.
    ///
    /// Used by `backspace()` to move the cursor back the correct distance.
    /// JetBrainsMono is monospaced so this value is constant after the first
    /// character is rendered.  Initialized to an estimate; updated on each
    /// `draw_char` call.
    last_advance: u32,
}

impl TextRenderer {
    pub fn new(
        point_size:  f32,
        foreground:  Color,
        background:  Color,
        margin_left: u32,
        margin_top:  u32,
    ) -> TextRenderer {
        let line_height = (point_size * 1.25) as u32;
        TextRenderer {
            cursor_x: margin_left,
            cursor_y: margin_top,
            point_size,
            foreground,
            background,
            margin_left,
            margin_top,
            line_height,
            last_advance: (point_size * 0.6) as u32,
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
        self.cursor_x = self.cursor_x.saturating_sub(self.last_advance).max(self.margin_left);
        // Erase the cell by filling it with the background colour.
        surface.fill_rect(
            self.cursor_x,
            self.cursor_y,
            self.last_advance,
            self.line_height,
            self.background.r,
            self.background.g,
            self.background.b,
        );
    }

    /// Draw a block cursor at the current cursor position.
    ///
    /// The cursor is a filled rectangle `last_advance` wide and `line_height`
    /// tall, drawn in the foreground colour.  Call `erase_cursor` before
    /// rendering new text so the cursor cell is cleared first.
    pub fn draw_cursor(&self, surface: &mut FramebufferSurface) {
        let width = self.last_advance.max(1);
        surface.fill_rect(
            self.cursor_x,
            self.cursor_y,
            width,
            self.line_height,
            self.foreground.r,
            self.foreground.g,
            self.foreground.b,
        );
    }

    /// Erase the cursor cell by filling it with the background colour.
    pub fn erase_cursor(&self, surface: &mut FramebufferSurface) {
        let width = self.last_advance.max(1);
        surface.fill_rect(
            self.cursor_x,
            self.cursor_y,
            width,
            self.line_height,
            self.background.r,
            self.background.g,
            self.background.b,
        );
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
    // Internal helpers
    // -----------------------------------------------------------------------

    fn draw_char(
        &mut self,
        character:    char,
        font_id:      FontId,
        font_manager: &mut FontManager,
        surface:      &mut FramebufferSurface,
    ) {
        let ascender = font_manager.ascender_pixels(font_id, self.point_size);
        let bitmap_opt = font_manager.rasterize(font_id, character, self.point_size);

        let advance = match bitmap_opt {
            Some(bitmap) => {
                let advance_pixels = bitmap.advance_width as u32;
                // Place glyph so its baseline aligns with cursor_y + ascender.
                //
                // fontdue coordinate system (y-up from baseline):
                //   bitmap row 0        = font-space y = ymin + height - 1  (topmost pixel)
                //   bitmap row height-1 = font-space y = ymin               (bottommost pixel)
                //   baseline            = font-space y = 0
                //
                // In screen space (y-down), the baseline is at cursor_y + ascender.
                //   glyph_top_screen = cursor_y + ascender - ymin - height + 1
                //
                // Note: ymin is signed (negative for descenders), so use i32 arithmetic.
                let ymin = bitmap.y_offset; // signed: negative for descenders
                let glyph_top = (self.cursor_y as i32
                    + ascender as i32
                    - ymin
                    - bitmap.height as i32
                    + 1)
                    .max(0) as u32;

                surface.draw_bitmap(
                    self.cursor_x,
                    glyph_top,
                    &bitmap.coverage,
                    bitmap.width,
                    bitmap.height,
                    self.foreground.r, self.foreground.g, self.foreground.b,
                    self.background.r, self.background.g, self.background.b,
                );

                advance_pixels
            }
            None => {
                // No glyph — advance by a fixed amount to avoid stalling.
                (self.point_size * 0.6) as u32
            }
        };

        self.last_advance = advance;
        self.cursor_x += advance;

        // Wrap at right edge.
        if self.cursor_x + advance > surface.width {
            self.newline(surface);
        }
    }
}
