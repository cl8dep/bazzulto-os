//! FramebufferSurface — userspace access to the boot-time framebuffer.
//!
//! Calls `sys_framebuffer_map` (syscall 70) once to map the physical framebuffer
//! pages into this process's address space, then exposes pixel-level drawing
//! primitives on top of that mapping.
//!
//! # Pixel format
//!
//! The Limine framebuffer is always 32 bpp.  The actual channel order varies
//! by platform (BGRX on most x86/AArch64 UEFI; RGBX on some).  The surface
//! reads the mask fields from the syscall response and packs pixels correctly
//! regardless of channel order.
//!
//! # Stride
//!
//! `stride` (bytes per row) may be greater than `width × 4` due to hardware
//! alignment requirements.  Always compute row offsets as `y × stride`, not
//! `y × width × 4`.

use bazzulto_system::raw;

// ---------------------------------------------------------------------------
// FramebufferSurface
// ---------------------------------------------------------------------------

/// Userspace representation of the boot-time framebuffer.
pub struct FramebufferSurface {
    /// Pointer to the first pixel of the framebuffer (user virtual address).
    base:   *mut u32,
    pub width:  u32,
    pub height: u32,
    /// Row stride in u32 units (stride_bytes / 4).
    stride_words: u32,
    /// Bit shift for each channel within a 32-bit pixel word.
    red_shift:   u32,
    green_shift: u32,
    blue_shift:  u32,
}

// SAFETY: The framebuffer mapping belongs to this process for its lifetime.
// No other Rust-level owner exists for the raw pointer.
unsafe impl Send for FramebufferSurface {}

/// Errors from `FramebufferSurface::open`.
#[derive(Debug)]
pub enum FramebufferError {
    /// The kernel has no framebuffer (headless or not yet initialised).
    NotAvailable,
    /// The kernel could not map the framebuffer into this process.
    MappingFailed,
}

impl FramebufferSurface {
    /// Map the boot-time framebuffer into this process and return a surface.
    ///
    /// Should be called once by the display server at startup.
    pub fn open() -> Result<FramebufferSurface, FramebufferError> {
        // out[0] = mapped_va, [1]=width, [2]=height, [3]=stride_bytes,
        // [4]=bpp, [5]=red_info, [6]=green_info, [7]=blue_info
        let mut descriptor = [0u64; 8];

        let result = raw::raw_framebuffer_map(descriptor.as_mut_ptr());
        if result < 0 {
            return Err(if result == -22 {
                FramebufferError::NotAvailable
            } else {
                FramebufferError::MappingFailed
            });
        }

        let mapped_va    = descriptor[0] as *mut u32;
        let width        = descriptor[1] as u32;
        let height       = descriptor[2] as u32;
        let stride_bytes = descriptor[3] as u32;
        // descriptor[4] = bpp (always 32 in practice)
        let red_shift   = (descriptor[5] & 0xFF) as u32;
        let green_shift = (descriptor[6] & 0xFF) as u32;
        let blue_shift  = (descriptor[7] & 0xFF) as u32;

        Ok(FramebufferSurface {
            base: mapped_va,
            width,
            height,
            stride_words: stride_bytes / 4,
            red_shift,
            green_shift,
            blue_shift,
        })
    }

    // -----------------------------------------------------------------------
    // Pixel primitives
    // -----------------------------------------------------------------------

    /// Write a single pixel at `(x, y)`.
    ///
    /// Out-of-bounds coordinates are silently ignored.
    #[inline]
    pub fn draw_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let word = self.pack(r, g, b);
        unsafe {
            let pixel_ptr = self.base.add((y * self.stride_words + x) as usize);
            core::ptr::write_volatile(pixel_ptr, word);
        }
    }

    /// Fill a rectangle with a solid colour.
    ///
    /// Coordinates are clamped to the surface bounds.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);
        let word  = self.pack(r, g, b);

        for row in y..y_end {
            for col in x..x_end {
                unsafe {
                    let pixel_ptr = self.base.add((row * self.stride_words + col) as usize);
                    core::ptr::write_volatile(pixel_ptr, word);
                }
            }
        }
    }

    /// Draw a glyph coverage bitmap at `(x, y)` with the given foreground colour
    /// blended over the given background colour using the coverage byte as alpha.
    ///
    /// `bitmap` is a row-major array of coverage bytes (0 = transparent, 255 = opaque).
    pub fn draw_bitmap(
        &mut self,
        x:       u32,
        y:       u32,
        bitmap:  &[u8],
        bm_width:  u32,
        bm_height: u32,
        fg_r: u8, fg_g: u8, fg_b: u8,
        bg_r: u8, bg_g: u8, bg_b: u8,
    ) {
        for row in 0..bm_height {
            let dst_y = y + row;
            if dst_y >= self.height {
                break;
            }
            for col in 0..bm_width {
                let dst_x = x + col;
                if dst_x >= self.width {
                    break;
                }
                let coverage = bitmap[(row * bm_width + col) as usize];
                let (r, g, b) = blend(coverage, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b);
                let word = self.pack(r, g, b);
                unsafe {
                    let pixel_ptr = self.base.add((dst_y * self.stride_words + dst_x) as usize);
                    core::ptr::write_volatile(pixel_ptr, word);
                }
            }
        }
    }

    /// Scroll the framebuffer up by `pixels` rows, filling the vacated region
    /// at the bottom with black.
    ///
    /// Uses a row-by-row copy rather than a flat memmove to respect the stride
    /// (which may differ from width × 4 due to hardware alignment).
    pub fn scroll_up(&mut self, pixels: u32) {
        if pixels == 0 || pixels >= self.height {
            self.clear();
            return;
        }
        // Copy rows [pixels .. height] to rows [0 .. height-pixels].
        for dst_row in 0..self.height.saturating_sub(pixels) {
            let src_row = dst_row + pixels;
            let src_offset = (src_row * self.stride_words) as usize;
            let dst_offset = (dst_row * self.stride_words) as usize;
            let row_words  = self.width as usize;
            unsafe {
                let src_ptr = self.base.add(src_offset);
                let dst_ptr = self.base.add(dst_offset);
                // SAFETY: src and dst are within the framebuffer mapping.
                // Rows never overlap because we always copy upward (dst < src).
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, row_words);
            }
        }
        // Clear the now-vacated bottom region.
        let cleared_from = self.height.saturating_sub(pixels);
        self.fill_rect(0, cleared_from, self.width, pixels, 0, 0, 0);
    }

    /// Clear the entire framebuffer to black.
    pub fn clear(&mut self) {
        self.fill_rect(0, 0, self.width, self.height, 0, 0, 0);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    #[inline(always)]
    fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        ((r as u32) << self.red_shift)
            | ((g as u32) << self.green_shift)
            | ((b as u32) << self.blue_shift)
    }
}

// ---------------------------------------------------------------------------
// Alpha blending helper (coverage × foreground + (1−coverage) × background)
// ---------------------------------------------------------------------------

/// Blend: `sharpened_coverage/255 × fg + (1 − sharpened_coverage/255) × bg`.
///
/// fontdue outputs linear greyscale coverage (0=transparent, 255=opaque).
/// On a low-DPI framebuffer the mid-grey anti-alias pixels look blurry, so we
/// apply a "screen"-mode sharpening curve before blending:
///
///   sharpened = 2c − c²/255     (equivalent to 1 − (1−c)²)
///
/// This maps:  0→0, 64→112, 128→192, 192→240, 255→255
/// Mid-grey values are pushed toward white, tightening the glyph edge and
/// reducing the blurry halo without eliminating anti-aliasing entirely.
///
/// Uses integer arithmetic only — no floating-point.
#[inline(always)]
fn blend(
    coverage: u8,
    fg_r: u8, fg_g: u8, fg_b: u8,
    bg_r: u8, bg_g: u8, bg_b: u8,
) -> (u8, u8, u8) {
    // Apply sharpening curve: sharpened = 2c − c²/255.
    let c = coverage as u32;
    let alpha = ((2 * c * 255 - c * c) / 255).min(255);
    let inv_alpha = 255 - alpha;

    let r = ((fg_r as u32 * alpha + bg_r as u32 * inv_alpha) / 255) as u8;
    let g = ((fg_g as u32 * alpha + bg_g as u32 * inv_alpha) / 255) as u8;
    let b = ((fg_b as u32 * alpha + bg_b as u32 * inv_alpha) / 255) as u8;

    (r, g, b)
}
