//! C ABI implementations for the functions declared in `display.h`.
//!
//! Each `bz_surface_t *` is a heap-allocated `Box<Surface>` cast to an opaque
//! pointer.  The caller owns the allocation; `bz_surface_destroy` drops it.
//!
//! `bz_screen_get` returns the same placeholder as `Screen::get()`.  In v2.0
//! it will read from the shared display-info page written by bzdisplayd.

use crate::color::Color;
use crate::geometry::Rect;
use crate::screen::Screen;
use crate::surface::Surface;

// ---------------------------------------------------------------------------
// Opaque handle
// ---------------------------------------------------------------------------

/// C-visible opaque type.  Only the pointer is ever exposed; the layout is
/// never visible to C callers.
#[repr(C)]
pub struct BzSurface {
    inner: Surface,
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

/// C layout must match `bz_screen_info_t` in display.h.
#[repr(C)]
pub struct BzScreenInfo {
    pub width:  u32,
    pub height: u32,
    pub dpi:    u32,
}

/// Fill `*info` with current display information.  Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn bz_screen_get(info: *mut BzScreenInfo) -> i32 {
    if info.is_null() {
        return -1;
    }
    let screen = Screen::get();
    (*info).width  = screen.resolution.width;
    (*info).height = screen.resolution.height;
    (*info).dpi    = screen.dpi;
    0
}

// ---------------------------------------------------------------------------
// Surface lifecycle
// ---------------------------------------------------------------------------

/// Allocate a new surface of the given dimensions.  Returns NULL on OOM.
#[no_mangle]
pub extern "C" fn bz_surface_create(width: u32, height: u32) -> *mut BzSurface {
    let boxed = alloc::boxed::Box::new(BzSurface {
        inner: Surface::new(width, height),
    });
    alloc::boxed::Box::into_raw(boxed)
}

/// Free a surface allocated by `bz_surface_create`.
///
/// # Safety
/// `surface` must have been returned by `bz_surface_create` and must not be
/// used after this call.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_destroy(surface: *mut BzSurface) {
    if !surface.is_null() {
        drop(alloc::boxed::Box::from_raw(surface));
    }
}

// ---------------------------------------------------------------------------
// Dimensions
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn bz_surface_width(surface: *const BzSurface) -> u32 {
    if surface.is_null() { return 0; }
    (*surface).inner.width
}

#[no_mangle]
pub unsafe extern "C" fn bz_surface_height(surface: *const BzSurface) -> u32 {
    if surface.is_null() { return 0; }
    (*surface).inner.height
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// C layout must match `bz_color_t` (u32 RGBA: bits [31:24]=R … [7:0]=A).
type BzColor = u32;

fn unpack_color(packed: BzColor) -> Color {
    Color {
        r: ((packed >> 24) & 0xFF) as u8,
        g: ((packed >> 16) & 0xFF) as u8,
        b: ((packed >>  8) & 0xFF) as u8,
        a: ( packed        & 0xFF) as u8,
    }
}

/// C layout must match `bz_rect_t`.
#[repr(C)]
pub struct BzRect {
    pub x:      i32,
    pub y:      i32,
    pub width:  u32,
    pub height: u32,
}

/// Write a single pixel.  Out-of-bounds coordinates are ignored.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_set_pixel(
    surface: *mut BzSurface,
    x: u32,
    y: u32,
    color: BzColor,
) {
    if surface.is_null() { return; }
    (*surface).inner.set_pixel(x, y, unpack_color(color));
}

/// Fill a rectangle with a solid color.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_fill_rect(
    surface: *mut BzSurface,
    rect: BzRect,
    color: BzColor,
) {
    if surface.is_null() { return; }
    let r = Rect::new(rect.x, rect.y, rect.width, rect.height);
    (*surface).inner.fill_rect(r, unpack_color(color));
}

/// Clear the entire surface to transparent black.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_clear(surface: *mut BzSurface) {
    if surface.is_null() { return; }
    (*surface).inner.clear();
}

/// Clear the entire surface to a solid color.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_clear_color(surface: *mut BzSurface, color: BzColor) {
    if surface.is_null() { return; }
    (*surface).inner.clear_color(unpack_color(color));
}

// ---------------------------------------------------------------------------
// Raw pixel access
// ---------------------------------------------------------------------------

/// Read-only pointer to the raw pixel data (width × height `bz_color_t` u32 values).
///
/// The pointer is valid until the surface is destroyed or any mutating call is
/// made.  C callers must not write through this pointer.
#[no_mangle]
pub unsafe extern "C" fn bz_surface_pixels(surface: *const BzSurface) -> *const u32 {
    if surface.is_null() { return core::ptr::null(); }
    (*surface).inner.pixels().as_ptr()
}
