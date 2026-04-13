//! Screen — read-only display information.
//!
//! Apps query `Screen::get()` to learn the display resolution and DPI.
//! This information comes from `sys_framebuffer_map` data made available
//! by the display server.  Apps do not call the syscall directly — they
//! receive screen info via a well-known shared region set up by bzdisplayd.
//!
//! # v1.0
//!
//! For now `Screen::get()` returns a placeholder.  In v2.0 bzdisplayd will
//! write the real values into a read-only MAP_SHARED page that all apps map.

use crate::geometry::Size;

/// Read-only display descriptor available to apps.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    /// Physical resolution of the display in pixels.
    pub resolution: Size,
    /// Dots per inch (logical). 96 is the baseline for 100% scaling.
    pub dpi: u32,
}

impl Screen {
    /// Return display information.
    ///
    /// In v1.0 this returns a static placeholder.  In v2.0 it reads from
    /// the shared display-info page written by bzdisplayd.
    pub fn get() -> Screen {
        Screen {
            resolution: Size::new(1920, 1080), // placeholder
            dpi: 96,
        }
    }

    /// Scaling factor relative to 96 DPI (1.0 = 100%).
    pub fn scale_factor(self) -> f32 {
        self.dpi as f32 / 96.0
    }
}
