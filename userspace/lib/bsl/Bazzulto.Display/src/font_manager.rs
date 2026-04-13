//! FontManager — loads, caches, and rasterizes TrueType fonts.
//!
//! # Font identity
//!
//! A font is identified by a **family name** (e.g. `"JetBrainsMono"`) plus a
//! **variant** (`FontVariant { weight, style }`).  This maps to a file on disk
//! using two search strategies (tried in order):
//!
//! 1. Flat:   `/system/fonts/<Family>-<Variant>.ttf`
//!            e.g. `/system/fonts/JetBrainsMono-BoldItalic.ttf`
//! 2. Grouped: `/system/fonts/<Family>/<Variant>.ttf`
//!            e.g. `/system/fonts/JetBrainsMono/BoldItalic.ttf`
//!
//! Both layouts are supported so font packages that ship flat files and those
//! that group variants in a sub-directory both work without configuration.
//!
//! # Glyph cache
//!
//! `GlyphCache` stores rasterized bitmaps keyed by `(FontId, codepoint,
//! size_fp)`.  `size_fp` is the point size × 64 (6 fractional bits), matching
//! the FreeType / fontdue convention to avoid floating-point key issues.

use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

use bazzulto_system::raw;

// ---------------------------------------------------------------------------
// Optional embedded fallback font
// ---------------------------------------------------------------------------

// To activate, place a TTF in the source tree and uncomment:
// pub static EMBEDDED_FALLBACK_FONT: Option<&[u8]> =
//     Some(include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf"));
pub static EMBEDDED_FALLBACK_FONT: Option<&[u8]> = None;

// ---------------------------------------------------------------------------
// Font search directories
// ---------------------------------------------------------------------------

/// Directories searched for system fonts, in priority order.
pub const SYSTEM_FONT_DIRECTORIES: &[&str] = &[
    "/system/fonts",
    "/usr/share/fonts",
    "/home/user/fonts",
];

// ---------------------------------------------------------------------------
// Font variant — weight + style
// ---------------------------------------------------------------------------

/// Typographic weight of a font variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontWeight {
    Thin,
    ExtraLight,
    Light,
    Regular,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Black,
}

/// Typographic style (posture) of a font variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

/// Combination of weight and style that identifies a single face within a
/// font family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FontVariant {
    pub weight: FontWeight,
    pub style:  FontStyle,
}

impl FontVariant {
    /// The canonical file-name suffix used for this variant.
    ///
    /// Follows the Google Fonts / JetBrains naming convention:
    ///   `Regular`, `Bold`, `Italic`, `BoldItalic`, `Light`, `LightItalic`, …
    pub fn filename_suffix(self) -> &'static str {
        match (self.weight, self.style) {
            (FontWeight::Thin,       FontStyle::Normal)  => "Thin",
            (FontWeight::Thin,       FontStyle::Italic)  => "ThinItalic",
            (FontWeight::Thin,       FontStyle::Oblique) => "ThinOblique",
            (FontWeight::ExtraLight, FontStyle::Normal)  => "ExtraLight",
            (FontWeight::ExtraLight, FontStyle::Italic)  => "ExtraLightItalic",
            (FontWeight::ExtraLight, FontStyle::Oblique) => "ExtraLightOblique",
            (FontWeight::Light,      FontStyle::Normal)  => "Light",
            (FontWeight::Light,      FontStyle::Italic)  => "LightItalic",
            (FontWeight::Light,      FontStyle::Oblique) => "LightOblique",
            (FontWeight::Regular,    FontStyle::Normal)  => "Regular",
            (FontWeight::Regular,    FontStyle::Italic)  => "Italic",
            (FontWeight::Regular,    FontStyle::Oblique) => "Oblique",
            (FontWeight::Medium,     FontStyle::Normal)  => "Medium",
            (FontWeight::Medium,     FontStyle::Italic)  => "MediumItalic",
            (FontWeight::Medium,     FontStyle::Oblique) => "MediumOblique",
            (FontWeight::SemiBold,   FontStyle::Normal)  => "SemiBold",
            (FontWeight::SemiBold,   FontStyle::Italic)  => "SemiBoldItalic",
            (FontWeight::SemiBold,   FontStyle::Oblique) => "SemiBoldOblique",
            (FontWeight::Bold,       FontStyle::Normal)  => "Bold",
            (FontWeight::Bold,       FontStyle::Italic)  => "BoldItalic",
            (FontWeight::Bold,       FontStyle::Oblique) => "BoldOblique",
            (FontWeight::ExtraBold,  FontStyle::Normal)  => "ExtraBold",
            (FontWeight::ExtraBold,  FontStyle::Italic)  => "ExtraBoldItalic",
            (FontWeight::ExtraBold,  FontStyle::Oblique) => "ExtraBoldOblique",
            (FontWeight::Black,      FontStyle::Normal)  => "Black",
            (FontWeight::Black,      FontStyle::Italic)  => "BlackItalic",
            (FontWeight::Black,      FontStyle::Oblique) => "BlackOblique",
        }
    }
}

impl Default for FontVariant {
    fn default() -> FontVariant {
        FontVariant {
            weight: FontWeight::Regular,
            style:  FontStyle::Normal,
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Numeric handle for a loaded font face.  Cheaper to copy than a String.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct FontId(pub u16);

/// Cache key: (font face, codepoint, point size in 1/64th-point units).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct GlyphKey {
    font_id:   FontId,
    codepoint: u32,
    /// Point size × 64, so 16.0 pt → 1024.  Integer key avoids f32 equality bugs.
    size_fp:   u32,
}

/// Rasterized glyph — alpha-coverage bitmap plus layout metrics.
pub struct GlyphBitmap {
    /// Coverage values, one byte per pixel (0 = transparent, 255 = fully opaque).
    pub coverage:      Vec<u8>,
    pub width:         u32,
    pub height:        u32,
    /// Horizontal distance to advance to the next glyph origin (pixels).
    pub advance_width: f32,
    /// Signed vertical offset from baseline to the top of the bitmap (pixels).
    pub y_offset:      i32,
}

/// A loaded font face — the parsed fontdue object plus its identity.
struct LoadedFace {
    family:  String,
    variant: FontVariant,
    font:    fontdue::Font,
}

// ---------------------------------------------------------------------------
// GlyphCache
// ---------------------------------------------------------------------------

struct GlyphCache {
    entries: BTreeMap<GlyphKey, GlyphBitmap>,
}

impl GlyphCache {
    fn new() -> GlyphCache {
        GlyphCache { entries: BTreeMap::new() }
    }

    fn get(&self, key: GlyphKey) -> Option<&GlyphBitmap> {
        self.entries.get(&key)
    }

    fn insert(&mut self, key: GlyphKey, bitmap: GlyphBitmap) {
        self.entries.insert(key, bitmap);
    }
}

// ---------------------------------------------------------------------------
// FontManager
// ---------------------------------------------------------------------------

pub struct FontManager {
    faces:            Vec<LoadedFace>,
    cache:            GlyphCache,
    fallback_font_id: Option<FontId>,
}

impl FontManager {
    /// Create a new FontManager.
    ///
    /// If `EMBEDDED_FALLBACK_FONT` is set the fallback face is loaded
    /// immediately so the system always has a renderable font from the first
    /// frame, even when `/system/fonts/` is not yet mounted.
    pub fn new() -> FontManager {
        let mut manager = FontManager {
            faces:            Vec::new(),
            cache:            GlyphCache::new(),
            fallback_font_id: None,
        };

        if let Some(font_data) = EMBEDDED_FALLBACK_FONT {
            if let Ok(font_id) = manager.load_from_bytes(
                "JetBrainsMono",
                FontVariant::default(),
                font_data,
            ) {
                manager.fallback_font_id = Some(font_id);
            }
        }

        manager
    }

    /// Load a font face by family name and variant, searching system font
    /// directories.
    ///
    /// Returns the `FontId` on success.  If the face is already loaded,
    /// returns the existing id without re-reading the file.
    ///
    /// # Search order
    ///
    /// For each directory in `SYSTEM_FONT_DIRECTORIES`:
    ///   1. `<dir>/<Family>-<Variant>.ttf`    (flat layout)
    ///   2. `<dir>/<Family>/<Variant>.ttf`    (grouped layout)
    pub fn load_font(
        &mut self,
        family:  &str,
        variant: FontVariant,
    ) -> Result<FontId, FontError> {
        // Return existing id if already loaded.
        for (index, face) in self.faces.iter().enumerate() {
            if face.family == family && face.variant == variant {
                return Ok(FontId(index as u16));
            }
        }

        let path = self.find_font_file(family, variant).ok_or(FontError::NotFound)?;
        let data = read_file(&path)?;
        self.load_from_bytes(family, variant, &data)
    }

    /// Load a font face from raw TTF bytes without touching the filesystem.
    ///
    /// Useful for embedded fallback fonts or for callers that obtained the
    /// data through another channel.
    pub fn load_font_bytes(
        &mut self,
        family:  &str,
        variant: FontVariant,
        data:    &[u8],
    ) -> Result<FontId, FontError> {
        // Return existing id if already loaded.
        for (index, face) in self.faces.iter().enumerate() {
            if face.family == family && face.variant == variant {
                return Ok(FontId(index as u16));
            }
        }
        self.load_from_bytes(family, variant, data)
    }

    /// Rasterize a single character at the given point size.
    ///
    /// Uses the glyph cache.  Falls back to the embedded fallback font if the
    /// requested font cannot produce a bitmap for the codepoint.
    pub fn rasterize(
        &mut self,
        font_id:    FontId,
        character:  char,
        point_size: f32,
    ) -> Option<&GlyphBitmap> {
        let size_fp     = (point_size * 64.0) as u32;
        let codepoint   = character as u32;
        let primary_key = GlyphKey { font_id, codepoint, size_fp };

        if self.cache.get(primary_key).is_some() {
            return self.cache.get(primary_key);
        }

        if let Some(bitmap) = self.rasterize_uncached(font_id, character, point_size) {
            self.cache.insert(primary_key, bitmap);
            return self.cache.get(primary_key);
        }

        // Try the fallback face.
        let fallback_id = self.fallback_font_id?;
        if fallback_id == font_id {
            return None;
        }

        let fallback_key = GlyphKey { font_id: fallback_id, codepoint, size_fp };
        if self.cache.get(fallback_key).is_some() {
            return self.cache.get(fallback_key);
        }

        if let Some(bitmap) = self.rasterize_uncached(fallback_id, character, point_size) {
            self.cache.insert(fallback_key, bitmap);
            return self.cache.get(fallback_key);
        }

        None
    }

    /// Return the fallback `FontId`, if one is loaded.
    pub fn fallback_font_id(&self) -> Option<FontId> {
        self.fallback_font_id
    }

    /// Ascender height in pixels for `font_id` at `point_size`.
    ///
    /// This is the distance from the baseline to the top of the tallest glyph.
    /// Returns 0 if the font or metrics are unavailable.
    pub fn ascender_pixels(&self, font_id: FontId, point_size: f32) -> u32 {
        let face = match self.faces.get(font_id.0 as usize) {
            Some(f) => f,
            None => return 0,
        };
        match face.font.horizontal_line_metrics(point_size) {
            Some(metrics) => (metrics.ascent as u32) + 1, // +1 rounds up conservatively
            None => (point_size * 0.8) as u32,
        }
    }

    /// Number of currently loaded font faces.
    pub fn font_count(&self) -> usize {
        self.faces.len()
    }

    /// Family name and variant of the face with the given id.
    pub fn font_info(&self, font_id: FontId) -> Option<(&str, FontVariant)> {
        self.faces
            .get(font_id.0 as usize)
            .map(|face| (face.family.as_str(), face.variant))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn load_from_bytes(
        &mut self,
        family:  &str,
        variant: FontVariant,
        data:    &[u8],
    ) -> Result<FontId, FontError> {
        let font = fontdue::Font::from_bytes(
            data,
            fontdue::FontSettings::default(),
        ).map_err(|_| FontError::ParseFailed)?;

        let font_id = FontId(self.faces.len() as u16);
        self.faces.push(LoadedFace {
            family:  family.to_string(),
            variant,
            font,
        });

        Ok(font_id)
    }

    /// Search for the font file using three layouts (tried in order):
    ///
    /// 1. Flat:            `<dir>/<Family>-<Variant>.ttf`
    /// 2. Grouped/short:   `<dir>/<Family>/<Variant>.ttf`
    /// 3. Grouped/full:    `<dir>/<Family>/<Family>-<Variant>.ttf`
    ///
    /// Layout 3 matches packages like JetBrains that ship files named
    /// `JetBrainsMono-Regular.ttf` inside a `JetBrainsMono/` subdirectory.
    fn find_font_file(&self, family: &str, variant: FontVariant) -> Option<String> {
        let suffix = variant.filename_suffix();

        for directory in SYSTEM_FONT_DIRECTORIES {
            // Strategy 1 — flat: <dir>/<Family>-<Variant>.ttf
            let mut flat_path = String::from(*directory);
            flat_path.push('/');
            flat_path.push_str(family);
            flat_path.push('-');
            flat_path.push_str(suffix);
            flat_path.push_str(".ttf");

            if file_exists(&flat_path) {
                return Some(flat_path);
            }

            // Strategy 2 — grouped/short: <dir>/<Family>/<Variant>.ttf
            let mut grouped_short_path = String::from(*directory);
            grouped_short_path.push('/');
            grouped_short_path.push_str(family);
            grouped_short_path.push('/');
            grouped_short_path.push_str(suffix);
            grouped_short_path.push_str(".ttf");

            if file_exists(&grouped_short_path) {
                return Some(grouped_short_path);
            }

            // Strategy 3 — grouped/full: <dir>/<Family>/<Family>-<Variant>.ttf
            // Matches packages like JetBrainsMono that ship
            // JetBrainsMono/JetBrainsMono-Regular.ttf.
            let mut grouped_full_path = String::from(*directory);
            grouped_full_path.push('/');
            grouped_full_path.push_str(family);
            grouped_full_path.push('/');
            grouped_full_path.push_str(family);
            grouped_full_path.push('-');
            grouped_full_path.push_str(suffix);
            grouped_full_path.push_str(".ttf");

            if file_exists(&grouped_full_path) {
                return Some(grouped_full_path);
            }
        }

        None
    }

    fn rasterize_uncached(
        &self,
        font_id:    FontId,
        character:  char,
        point_size: f32,
    ) -> Option<GlyphBitmap> {
        let face = self.faces.get(font_id.0 as usize)?;
        let (metrics, bitmap) = face.font.rasterize(character, point_size);

        if metrics.width == 0 || metrics.height == 0 {
            return None;
        }

        Some(GlyphBitmap {
            coverage:      bitmap,
            width:         metrics.width  as u32,
            height:        metrics.height as u32,
            advance_width: metrics.advance_width,
            y_offset:      metrics.ymin,
        })
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FontError {
    /// No font file found in any search directory under any layout.
    NotFound,
    /// The file exists but fontdue could not parse it.
    ParseFailed,
    /// A filesystem read failed.
    IoError,
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn file_exists(path: &str) -> bool {
    let fd = raw::raw_open(path.as_ptr(), path.len());
    if fd >= 0 {
        raw::raw_close(fd as i32);
        true
    } else {
        false
    }
}

pub fn read_file(path: &str) -> Result<Vec<u8>, FontError> {
    let fd = raw::raw_open(path.as_ptr(), path.len());
    if fd < 0 {
        return Err(FontError::IoError);
    }
    let fd = fd as i32;

    let mut buffer = Vec::new();
    let mut chunk  = [0u8; 4096];
    loop {
        let bytes_read = raw::raw_read(fd, chunk.as_mut_ptr(), chunk.len());
        if bytes_read <= 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..bytes_read as usize]);
    }

    raw::raw_close(fd);
    Ok(buffer)
}
