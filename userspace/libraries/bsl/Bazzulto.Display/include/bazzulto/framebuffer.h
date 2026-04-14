#pragma once
/**
 * @file framebuffer.h
 * @brief Bazzulto.Display — direct framebuffer access (display server only).
 *
 * This API is intended exclusively for the Bazzulto display server
 * (bzdisplayd).  Normal applications must NOT call these functions; they will
 * receive EPERM unless they hold the CAP_DISPLAY capability.
 *
 * Usage:
 *   1. Call bz_framebuffer_map() once at startup.
 *   2. Write pixels to the mapped address directly as a packed array of
 *      uint32_t values in BGRX (or RGBX) order depending on the channel info
 *      fields returned in the descriptor.
 *   3. Pixels are row-major, left-to-right, top-to-bottom.
 *      Byte offset of pixel (x, y) = y × stride_bytes + x × (bpp / 8).
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Framebuffer descriptor populated by bz_framebuffer_map().
 *
 * All fields are read-only after the call succeeds.  The @c pixels pointer
 * is valid for the lifetime of the process.
 */
typedef struct {
    uint32_t  *pixels;        /**< Read-write pointer to the first pixel. */
    uint32_t   width;         /**< Horizontal resolution in pixels. */
    uint32_t   height;        /**< Vertical resolution in pixels. */
    uint32_t   stride_bytes;  /**< Bytes per row (≥ width × bpp / 8). */
    uint32_t   bpp;           /**< Bits per pixel (always 32 in practice). */
    uint8_t    red_shift;     /**< Bit position of the red channel LSB. */
    uint8_t    red_size;      /**< Number of bits in the red channel. */
    uint8_t    green_shift;   /**< Bit position of the green channel LSB. */
    uint8_t    green_size;    /**< Number of bits in the green channel. */
    uint8_t    blue_shift;    /**< Bit position of the blue channel LSB. */
    uint8_t    blue_size;     /**< Number of bits in the blue channel. */
    uint8_t    _pad[2];       /**< Reserved — always zero. */
} bz_framebuffer_t;

/**
 * @brief Map the boot framebuffer into the calling process's address space.
 *
 * Requires the @c CAP_DISPLAY capability.  Returns @c -EPERM if the process
 * does not hold it.
 *
 * On success, writes the following values into the 8-element @c uint64_t
 * array at @p raw_out and also fills @p fb_out (if non-NULL):
 *
 *   raw_out[0] = mapped virtual address (user read-write)
 *   raw_out[1] = width  (pixels)
 *   raw_out[2] = height (pixels)
 *   raw_out[3] = stride (bytes per row)
 *   raw_out[4] = bpp    (bits per pixel)
 *   raw_out[5] = red   channel info: (mask_size << 8) | mask_shift
 *   raw_out[6] = green channel info: (mask_size << 8) | mask_shift
 *   raw_out[7] = blue  channel info: (mask_size << 8) | mask_shift
 *
 * @param raw_out  8-element uint64_t output array — kernel ABI.
 * @param fb_out   Optional bz_framebuffer_t to fill from raw_out.  May be NULL.
 * @return 0 on success, -BZ_EINVAL if the framebuffer is unavailable,
 *         -BZ_ENOMEM if mapping fails, -BZ_EPERM if unauthorized.
 */
int64_t bz_framebuffer_map(uint64_t raw_out[8], bz_framebuffer_t *fb_out);

/**
 * @brief Helper: pack an RGBA color into the pixel format described by @p fb.
 *
 * @param fb  Framebuffer descriptor (from bz_framebuffer_map).
 * @param r   Red component (0–255).
 * @param g   Green component (0–255).
 * @param b   Blue component (0–255).
 * @return Packed pixel value suitable for writing to fb->pixels.
 */
static inline uint32_t bz_framebuffer_pack_rgb(const bz_framebuffer_t *fb,
                                               uint8_t r, uint8_t g, uint8_t b) {
    return ((uint32_t)r << fb->red_shift)
         | ((uint32_t)g << fb->green_shift)
         | ((uint32_t)b << fb->blue_shift);
}

/**
 * @brief Write a single pixel to the framebuffer (bounds-checked).
 *
 * @param fb     Framebuffer descriptor.
 * @param x      Pixel column (0-based).
 * @param y      Pixel row (0-based).
 * @param pixel  Packed pixel value (from bz_framebuffer_pack_rgb).
 */
static inline void bz_framebuffer_set_pixel(const bz_framebuffer_t *fb,
                                            uint32_t x, uint32_t y,
                                            uint32_t pixel) {
    if (x >= fb->width || y >= fb->height) return;
    uint32_t *row = (uint32_t *)((uint8_t *)fb->pixels + y * fb->stride_bytes);
    row[x] = pixel;
}

#ifdef __cplusplus
} /* extern "C" */
#endif
