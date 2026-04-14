//! Surface — the drawing canvas for apps.
//!
//! A `Surface` is an RGBA pixel buffer owned by the app.  The display server
//! reads from it (via MAP_SHARED) and composites it onto the physical
//! framebuffer.
//!
//! # v1.0
//!
//! `Surface` is a simple heap-allocated buffer.  Pixel writes go directly
//! into the allocation; in v2.0 this will be a MAP_SHARED region so the
//! display server can read it without a copy.
//!
//! # Pixel format
//!
//! Pixels are stored as `u32` values in RGBA byte order:
//!   bits [31:24] = R, [23:16] = G, [15:8] = B, [7:0] = A

use alloc::vec::Vec;
use crate::color::Color;
use crate::geometry::Rect;

pub struct Surface {
    pixels: Vec<u32>,
    pub width:  u32,
    pub height: u32,
}

impl Surface {
    /// Allocate a new surface filled with transparent black.
    pub fn new(width: u32, height: u32) -> Surface {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        Surface {
            pixels: alloc::vec![0u32; pixel_count],
            width,
            height,
        }
    }

    /// Raw pixel slice (RGBA u32 values, row-major).
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Write a single pixel.  Out-of-bounds coordinates are ignored.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = (y * self.width + x) as usize;
        self.pixels[index] = pack_rgba(color);
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x_start = rect.x().max(0) as u32;
        let y_start = rect.y().max(0) as u32;
        let x_end   = (rect.right().max(0) as u32).min(self.width);
        let y_end   = (rect.bottom().max(0) as u32).min(self.height);
        let word    = pack_rgba(color);

        for row in y_start..y_end {
            let row_start = (row * self.width + x_start) as usize;
            let row_end   = (row * self.width + x_end)   as usize;
            self.pixels[row_start..row_end].fill(word);
        }
    }

    /// Clear the entire surface to transparent black.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Clear to a solid color.
    pub fn clear_color(&mut self, color: Color) {
        self.pixels.fill(pack_rgba(color));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline(always)]
fn pack_rgba(c: Color) -> u32 {
    ((c.r as u32) << 24)
        | ((c.g as u32) << 16)
        | ((c.b as u32) << 8)
        | (c.a as u32)
}
