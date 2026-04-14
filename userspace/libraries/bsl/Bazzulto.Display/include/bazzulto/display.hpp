#pragma once
// Bazzulto.Display — drawing API (C++ API)
//
// Thin RAII wrappers over the C ABI in display.h.
// All types live in the Bazzulto namespace.

#include <bazzulto/display.h>
#include <cstdint>
#include <cstddef>

namespace Bazzulto {

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

/// 8-bit RGBA color value.
struct Color {
    uint8_t r, g, b, a;

    constexpr Color(uint8_t r, uint8_t g, uint8_t b, uint8_t a = 255) noexcept
        : r(r), g(g), b(b), a(a) {}

    /// Convert to the packed C representation used by the surface API.
    constexpr bz_color_t pack() const noexcept {
        return bz_rgba(r, g, b, a);
    }

    static constexpr Color black()       noexcept { return {   0,   0,   0 }; }
    static constexpr Color white()       noexcept { return { 255, 255, 255 }; }
    static constexpr Color red()         noexcept { return { 255,   0,   0 }; }
    static constexpr Color green()       noexcept { return {   0, 255,   0 }; }
    static constexpr Color blue()        noexcept { return {   0,   0, 255 }; }
    static constexpr Color cyan()        noexcept { return {   0, 255, 255 }; }
    static constexpr Color yellow()      noexcept { return { 255, 255,   0 }; }
    static constexpr Color gray()        noexcept { return { 128, 128, 128 }; }
    static constexpr Color transparent() noexcept { return {   0,   0,   0, 0 }; }
};

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

struct Point {
    int32_t x = 0;
    int32_t y = 0;

    constexpr Point(int32_t x = 0, int32_t y = 0) noexcept : x(x), y(y) {}

    constexpr bz_point_t c() const noexcept { return { x, y }; }
};

struct Size {
    uint32_t width  = 0;
    uint32_t height = 0;

    constexpr Size(uint32_t w = 0, uint32_t h = 0) noexcept : width(w), height(h) {}

    constexpr bz_size_t c() const noexcept { return { width, height }; }
    constexpr bool is_empty()  const noexcept { return width == 0 || height == 0; }
    constexpr uint64_t area()  const noexcept { return uint64_t(width) * height; }
};

struct Rect {
    int32_t  x      = 0;
    int32_t  y      = 0;
    uint32_t width  = 0;
    uint32_t height = 0;

    constexpr Rect(int32_t x = 0, int32_t y = 0,
                   uint32_t w = 0, uint32_t h = 0) noexcept
        : x(x), y(y), width(w), height(h) {}

    constexpr bz_rect_t c() const noexcept { return { x, y, width, height }; }

    constexpr int32_t right()  const noexcept { return x + int32_t(width); }
    constexpr int32_t bottom() const noexcept { return y + int32_t(height); }
    constexpr bool is_empty()  const noexcept { return width == 0 || height == 0; }

    constexpr bool contains(Point p) const noexcept {
        return p.x >= x && p.x < right() && p.y >= y && p.y < bottom();
    }
};

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

/// Read-only display information.
struct Screen {
    uint32_t width;
    uint32_t height;
    uint32_t dpi;

    /// Query the current display. Returns a default Screen on error.
    static Screen get() noexcept {
        bz_screen_info_t info{};
        bz_screen_get(&info);
        return { info.width, info.height, info.dpi };
    }

    /// Logical scaling factor relative to 96 DPI (1.0 = 100%).
    float scale_factor() const noexcept {
        return float(dpi) / 96.0f;
    }

    Size resolution() const noexcept { return { width, height }; }
};

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// RAII owner of an app drawing surface.
///
/// Wraps `bz_surface_t*`. Not copyable; moveable.
/// The display server reads the pixel buffer to composite it onto the screen.
class Surface {
public:
    /// Create a surface of the given dimensions. Check `valid()` after construction.
    Surface(uint32_t width, uint32_t height) noexcept
        : handle_(bz_surface_create(width, height)) {}

    ~Surface() noexcept {
        if (handle_) bz_surface_destroy(handle_);
    }

    // Non-copyable.
    Surface(const Surface&) = delete;
    Surface& operator=(const Surface&) = delete;

    // Moveable.
    Surface(Surface&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }
    Surface& operator=(Surface&& other) noexcept {
        if (this != &other) {
            if (handle_) bz_surface_destroy(handle_);
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    bool valid() const noexcept { return handle_ != nullptr; }

    uint32_t width()  const noexcept { return bz_surface_width(handle_); }
    uint32_t height() const noexcept { return bz_surface_height(handle_); }
    Size     size()   const noexcept { return { width(), height() }; }

    void set_pixel(uint32_t x, uint32_t y, Color color) noexcept {
        bz_surface_set_pixel(handle_, x, y, color.pack());
    }

    void fill_rect(Rect rect, Color color) noexcept {
        bz_surface_fill_rect(handle_, rect.c(), color.pack());
    }

    void clear() noexcept {
        bz_surface_clear(handle_);
    }

    void clear(Color color) noexcept {
        bz_surface_clear_color(handle_, color.pack());
    }

    /// Raw read-only access to pixel data (width × height u32 values, RGBA).
    const uint32_t* pixels() const noexcept {
        return bz_surface_pixels(handle_);
    }

    /// Underlying C handle — use only when calling C APIs directly.
    bz_surface_t* handle() const noexcept { return handle_; }

private:
    bz_surface_t* handle_;
};

} // namespace Bazzulto
