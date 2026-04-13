#pragma once
/** @file display.h
 *  @brief Bazzulto.Display — drawing API, C ABI
 *
 *  Apps draw into a Surface (their own pixel buffer). The display server
 *  reads from it and composites it onto the physical framebuffer.
 *  Apps never access the framebuffer directly.
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/** @defgroup color Color
 *  8-bit RGBA packed as 0xRRGGBBAA.
 *  @{
 */

/** @brief Packed RGBA color value (0xRRGGBBAA). */
typedef uint32_t bz_color_t;

/** @brief Construct a color from RGBA components.
 *  @param r Red channel (0–255).
 *  @param g Green channel (0–255).
 *  @param b Blue channel (0–255).
 *  @param a Alpha channel (0=transparent, 255=opaque).
 *  @return Packed bz_color_t value.
 */
static inline bz_color_t bz_rgba(uint8_t r, uint8_t g, uint8_t b, uint8_t a) {
    return ((uint32_t)r << 24)
         | ((uint32_t)g << 16)
         | ((uint32_t)b <<  8)
         | ((uint32_t)a);
}

/** @brief Construct an opaque color from RGB components.
 *  @param r Red channel (0–255).
 *  @param g Green channel (0–255).
 *  @param b Blue channel (0–255).
 *  @return Packed bz_color_t value with alpha=255.
 */
static inline bz_color_t bz_rgb(uint8_t r, uint8_t g, uint8_t b) {
    return bz_rgba(r, g, b, 255);
}

#define BZ_COLOR_BLACK       bz_rgb(0,   0,   0)    /**< Opaque black. */
#define BZ_COLOR_WHITE       bz_rgb(255, 255, 255)  /**< Opaque white. */
#define BZ_COLOR_RED         bz_rgb(255, 0,   0)    /**< Opaque red. */
#define BZ_COLOR_GREEN       bz_rgb(0,   255, 0)    /**< Opaque green. */
#define BZ_COLOR_BLUE        bz_rgb(0,   0,   255)  /**< Opaque blue. */
#define BZ_COLOR_CYAN        bz_rgb(0,   255, 255)  /**< Opaque cyan. */
#define BZ_COLOR_YELLOW      bz_rgb(255, 255, 0)    /**< Opaque yellow. */
#define BZ_COLOR_GRAY        bz_rgb(128, 128, 128)  /**< Opaque mid-gray. */
#define BZ_COLOR_TRANSPARENT bz_rgba(0, 0, 0, 0)    /**< Fully transparent black. */
/** @} */

/** @defgroup geometry Geometry Types
 *  @{
 */

/** @brief A 2D point with signed integer coordinates. */
typedef struct {
    int32_t x;  /**< Horizontal position, in pixels. */
    int32_t y;  /**< Vertical position, in pixels. */
} bz_point_t;

/** @brief A 2D size with unsigned integer dimensions. */
typedef struct {
    uint32_t width;   /**< Width in pixels. */
    uint32_t height;  /**< Height in pixels. */
} bz_size_t;

/** @brief An axis-aligned rectangle. */
typedef struct {
    int32_t  x;       /**< Left edge, in pixels. */
    int32_t  y;       /**< Top edge, in pixels. */
    uint32_t width;   /**< Width in pixels. */
    uint32_t height;  /**< Height in pixels. */
} bz_rect_t;

/** @brief Construct a bz_rect_t from components.
 *  @param x  Left edge.
 *  @param y  Top edge.
 *  @param w  Width.
 *  @param h  Height.
 */
static inline bz_rect_t bz_rect(int32_t x, int32_t y, uint32_t w, uint32_t h) {
    bz_rect_t r = { x, y, w, h };
    return r;
}
/** @} */

/** @defgroup screen Screen
 *  Read-only display information.
 *  @{
 */

/** @brief Physical display properties. */
typedef struct {
    uint32_t width;   /**< Screen width in pixels. */
    uint32_t height;  /**< Screen height in pixels. */
    uint32_t dpi;     /**< Dots per inch. */
} bz_screen_info_t;

/**
 * @brief Query the current display properties.
 * @param info  Pointer where the display information is written.
 * @return 0 on success.
 */
int32_t bz_screen_get(bz_screen_info_t *info);
/** @} */

/** @defgroup surface Surface
 *  The application's drawing canvas.
 *
 *  A surface is an RGBA pixel buffer. Pixels are @c bz_color_t values (RGBA u32)
 *  stored row-major, left-to-right, top-to-bottom.
 *  @{
 */

/** @brief Opaque surface handle. */
typedef struct bz_surface bz_surface_t;

/**
 * @brief Allocate a new surface.
 * @param width   Surface width in pixels.
 * @param height  Surface height in pixels.
 * @return Pointer to the new surface, or NULL on allocation failure.
 */
bz_surface_t *bz_surface_create(uint32_t width, uint32_t height);

/**
 * @brief Free a surface and its pixel buffer.
 * @param surface  Surface to destroy.
 */
void bz_surface_destroy(bz_surface_t *surface);

/** @brief Return the width of a surface in pixels. */
uint32_t bz_surface_width(const bz_surface_t *surface);

/** @brief Return the height of a surface in pixels. */
uint32_t bz_surface_height(const bz_surface_t *surface);

/**
 * @brief Write a single pixel. Out-of-bounds coordinates are ignored.
 * @param surface  Target surface.
 * @param x        Pixel column.
 * @param y        Pixel row.
 * @param color    Color to write.
 */
void bz_surface_set_pixel(bz_surface_t *surface,
                          uint32_t x, uint32_t y,
                          bz_color_t color);

/**
 * @brief Fill a rectangle with a solid color.
 * @param surface  Target surface.
 * @param rect     Rectangle to fill. Clipped to the surface bounds.
 * @param color    Fill color.
 */
void bz_surface_fill_rect(bz_surface_t *surface,
                          bz_rect_t rect,
                          bz_color_t color);

/**
 * @brief Clear the entire surface to transparent black (0x00000000).
 * @param surface  Target surface.
 */
void bz_surface_clear(bz_surface_t *surface);

/**
 * @brief Clear the entire surface to a solid color.
 * @param surface  Target surface.
 * @param color    Fill color.
 */
void bz_surface_clear_color(bz_surface_t *surface, bz_color_t color);

/**
 * @brief Return a read-only pointer to the raw pixel data.
 *
 * The buffer contains @c width × @c height @c bz_color_t values,
 * stored row-major, left-to-right, top-to-bottom.
 *
 * @param surface  Source surface.
 * @return Pointer to the pixel buffer.
 */
const uint32_t *bz_surface_pixels(const bz_surface_t *surface);
/** @} */

#ifdef __cplusplus
} /* extern "C" */
#endif
