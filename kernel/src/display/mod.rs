// display/mod.rs — Kernel-side framebuffer info for sys_framebuffer_map.
//
// The framebuffer address provided by Limine is a virtual address in the
// kernel's HHDM mapping.  We derive and store the physical base at init time
// so the syscall handler can map those pages into any user process.
//
// Called once from kernel_main() after the Limine framebuffer response is
// validated.  Safe to read from any context thereafter (single-core, no
// concurrent mutation).

use core::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Stored framebuffer descriptor
// ---------------------------------------------------------------------------

/// Kernel representation of the boot-time framebuffer.
pub struct KernelFramebufferInfo {
    /// Physical base address of the framebuffer (derived from Limine virtual +
    /// HHDM offset at store time).
    pub phys_base: u64,
    /// Total size in bytes (pitch × height), page-rounded up by the caller.
    pub size_bytes: u64,
    pub width:  u64,
    pub height: u64,
    /// Row stride in bytes (may be > width × bpp/8 due to hardware alignment).
    pub stride: u64,
    /// Bits per pixel (typically 32).
    pub bpp: u16,
    pub red_mask_size:    u8,
    pub red_mask_shift:   u8,
    pub green_mask_size:  u8,
    pub green_mask_shift: u8,
    pub blue_mask_size:   u8,
    pub blue_mask_shift:  u8,
}

// SAFETY: single-core kernel; written once before any process runs.
unsafe impl Sync for KernelFramebufferInfo {}
unsafe impl Send for KernelFramebufferInfo {}

static mut FRAMEBUFFER_INFO: KernelFramebufferInfo = KernelFramebufferInfo {
    phys_base:        0,
    size_bytes:       0,
    width:            0,
    height:           0,
    stride:           0,
    bpp:              0,
    red_mask_size:    0,
    red_mask_shift:   0,
    green_mask_size:  0,
    green_mask_shift: 0,
    blue_mask_size:   0,
    blue_mask_shift:  0,
};

static FRAMEBUFFER_READY: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Initialisation — called from kernel_main
// ---------------------------------------------------------------------------

/// Store the framebuffer descriptor for later use by sys_framebuffer_map.
///
/// `virtual_base` is the address from Limine's Framebuffer.address (HHDM VA).
/// `hhdm_offset` is the HHDM offset reported by Limine.
///
/// # Safety
/// Must be called exactly once, before any process is spawned.
pub unsafe fn store(
    virtual_base: *mut u32,
    width:        u64,
    height:       u64,
    stride:       u64,
    bpp:          u16,
    red_size:     u8,
    red_shift:    u8,
    green_size:   u8,
    green_shift:  u8,
    blue_size:    u8,
    blue_shift:   u8,
    hhdm_offset:  u64,
) {
    let phys_base = (virtual_base as u64).wrapping_sub(hhdm_offset);
    let size_bytes = stride * height;

    FRAMEBUFFER_INFO = KernelFramebufferInfo {
        phys_base,
        size_bytes,
        width,
        height,
        stride,
        bpp,
        red_mask_size:    red_size,
        red_mask_shift:   red_shift,
        green_mask_size:  green_size,
        green_mask_shift: green_shift,
        blue_mask_size:   blue_size,
        blue_mask_shift:  blue_shift,
    };

    FRAMEBUFFER_READY.store(true, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Accessor — called from sys_framebuffer_map
// ---------------------------------------------------------------------------

/// Return a reference to the stored framebuffer info, or None if not yet
/// initialised.
pub fn get() -> Option<&'static KernelFramebufferInfo> {
    if FRAMEBUFFER_READY.load(Ordering::Acquire) {
        Some(unsafe { &FRAMEBUFFER_INFO })
    } else {
        None
    }
}
