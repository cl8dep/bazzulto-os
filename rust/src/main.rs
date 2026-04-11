//! Bazzulto OS — Rust kernel entry point
//!
//! This is the Rust rewrite of the Bazzulto AArch64 kernel.
//! Boot path: QEMU virt → UEFI → Limine → _start (start.S) → kernel_main (here)

#![no_std]
#![no_main]

mod arch;
mod drivers;
mod limine;

use drivers::{console, uart};

// ---------------------------------------------------------------------------
// Limine requests
//
// Placed in .limine_requests so the bootloader can locate them.
// #[used] prevents the compiler from discarding them as dead code.
// ---------------------------------------------------------------------------

#[used]
#[link_section = ".limine_requests_start"]
static LIMINE_REQUESTS_START: [u64; 4] = [
    0xf6b8f4b39de7d1ae,
    0xfab91a6940fcb9cf,
    0x785c6ed015d3e316,
    0x181e920a7852b9d9,
];

#[used]
#[link_section = ".limine_requests_end"]
static LIMINE_REQUESTS_END: [u64; 2] = [0xadc0e0531bb10d03, 0x9572709f31764c62];

// Limine request objects MUST be in a writable section.
// Limine writes the response pointer into the `response` field at boot time.
// Using `static mut` ensures the compiler emits a writable section (W flag in ELF).
// All accesses are wrapped in `unsafe`; this is safe because Limine writes once
// before handing control to the kernel, and the kernel is single-core at this point.

#[used]
#[link_section = ".limine_requests"]
static mut FRAMEBUFFER_REQUEST: limine::FramebufferRequest = limine::FramebufferRequest {
    id: limine::FRAMEBUFFER_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut HHDM_REQUEST: limine::HhdmRequest = limine::HhdmRequest {
    id: limine::HHDM_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut MEMMAP_REQUEST: limine::MemmapRequest = limine::MemmapRequest {
    id: limine::MEMMAP_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut KERNEL_ADDRESS_REQUEST: limine::KernelAddressRequest = limine::KernelAddressRequest {
    id: limine::KERNEL_ADDRESS_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut BOOTLOADER_INFO_REQUEST: limine::BootloaderInfoRequest =
    limine::BootloaderInfoRequest {
        id: limine::BOOTLOADER_INFO_REQUEST_ID,
        revision: 0,
        response: core::ptr::null_mut(),
    };

// ---------------------------------------------------------------------------
// Panic handler — no unwinding in a freestanding kernel
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    // At this point we cannot guarantee the console is up, so also send to UART.
    uart::puts("KERNEL PANIC: ");
    if let Some(msg) = info.message().as_str() {
        uart::puts(msg);
    }
    uart::puts("\r\n");

    console::print_str("KERNEL PANIC: ");
    if let Some(msg) = info.message().as_str() {
        console::print_str(msg);
    }
    console::print_str("\n");

    halt()
}

fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

// ---------------------------------------------------------------------------
// kernel_main — called from start.S via `bl kernel_main`
// ---------------------------------------------------------------------------

/// Main kernel entry point.
///
/// Called by `_start` in `src/arch/arm64/boot/start.S` after the stack is set up.
/// `extern "C"` ensures the symbol name is not mangled and the calling convention
/// matches what the assembler `bl kernel_main` instruction expects (AAPCS64).
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // --- Step 0: get HHDM offset (needed by UART and console) ---
    let hhdm_offset = unsafe {
        if HHDM_REQUEST.response.is_null() {
            halt();
        }
        (*HHDM_REQUEST.response).offset
    };

    // --- Step 1: initialise early UART output ---
    uart::early_init(hhdm_offset);
    uart::puts("Bazzulto OS (Rust) booting...\r\n");

    // --- Step 2: initialise framebuffer console ---
    let fb = unsafe {
        if FRAMEBUFFER_REQUEST.response.is_null() {
            uart::puts("FATAL: no framebuffer response\r\n");
            halt();
        }
        let resp = &*FRAMEBUFFER_REQUEST.response;
        if resp.framebuffer_count < 1 {
            uart::puts("FATAL: no framebuffers\r\n");
            halt();
        }
        &**resp.framebuffers
    };

    unsafe {
        console::init(fb.address, fb.width, fb.height, fb.pitch);
    }

    console::println("Bazzulto OS (Rust) booting...");

    // --- Step 3: print bootloader info ---
    unsafe {
        if !BOOTLOADER_INFO_REQUEST.response.is_null() {
            let resp = &*BOOTLOADER_INFO_REQUEST.response;
            console::print_str("Bootloader: ");
            console::print_str(cstr_to_str(resp.name));
            console::print_str(" ");
            console::println(cstr_to_str(resp.version));
        }
    }

    // --- Step 4: print HHDM info ---
    {
        use core::fmt::Write;
        let _ = writeln!(console::ConsoleWriter, "HHDM offset: {:#018x}", hhdm_offset);
    }

    // --- Step 5: count usable physical memory ---
    let usable_pages = unsafe {
        if MEMMAP_REQUEST.response.is_null() {
            uart::puts("FATAL: no memory map\r\n");
            halt();
        }
        let resp = &*MEMMAP_REQUEST.response;
        let mut pages: u64 = 0;
        for i in 0..resp.entry_count as usize {
            let entry = &**resp.entries.add(i);
            if entry.entry_type == limine::MEMMAP_USABLE {
                pages += entry.length / 4096;
            }
        }
        pages
    };

    {
        use core::fmt::Write;
        let _ = writeln!(
            console::ConsoleWriter,
            "Physical memory: {} pages usable",
            usable_pages
        );
    }

    // --- Step 6: print kernel load address ---
    unsafe {
        if !KERNEL_ADDRESS_REQUEST.response.is_null() {
            let resp = &*KERNEL_ADDRESS_REQUEST.response;
            use core::fmt::Write;
            let _ = writeln!(
                console::ConsoleWriter,
                "Kernel: phys {:#018x}  virt {:#018x}",
                resp.physical_base, resp.virtual_base
            );
        }
    }

    console::println("Phase 1 complete — framebuffer + UART operational.");
    uart::puts("Phase 1 complete.\r\n");

    halt()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a NUL-terminated C string pointer to a Rust &str.
///
/// # Safety
/// `ptr` must point to a valid NUL-terminated byte sequence.
unsafe fn cstr_to_str(ptr: *const u8) -> &'static str {
    if ptr.is_null() {
        return "<null>";
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
}
