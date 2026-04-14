//! Color types.

/// 8-bit-per-channel RGBA color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub const BLACK:       Color = Color::rgb(0, 0, 0);
    pub const WHITE:       Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const RED:         Color = Color::rgb(255, 0, 0);
    pub const GREEN:       Color = Color::rgb(0, 255, 0);
    pub const BLUE:        Color = Color::rgb(0, 0, 255);
    pub const GRAY:        Color = Color::rgb(128, 128, 128);
    pub const LIGHT_GRAY:  Color = Color::rgb(200, 200, 200);
    pub const DARK_GRAY:   Color = Color::rgb(64, 64, 64);
    pub const CYAN:        Color = Color::rgb(0, 255, 255);
    pub const YELLOW:      Color = Color::rgb(255, 255, 0);
    pub const MAGENTA:     Color = Color::rgb(255, 0, 255);
}

/// 32-bit float RGBA color — useful for blending and gradients.
#[derive(Clone, Copy, Debug)]
pub struct ColorF {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ColorF {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> ColorF {
        ColorF { r, g, b, a }
    }
}

impl From<Color> for ColorF {
    fn from(c: Color) -> ColorF {
        ColorF {
            r: c.r as f32 / 255.0,
            g: c.g as f32 / 255.0,
            b: c.b as f32 / 255.0,
            a: c.a as f32 / 255.0,
        }
    }
}
