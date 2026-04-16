//! bzdisplayd — Bazzulto Display Server
//!
//! # v1.0 responsibilities
//!
//! 1. Call `sys_framebuffer_map` to obtain exclusive read-write access to the
//!    boot-time framebuffer.
//! 2. Initialise `FontManager` and load system fonts.
//! 3. Render an initial splash / status line to prove the stack works.
//! 4. Stay resident — future versions will accept drawing requests from apps
//!    via MAP_SHARED backbuffers + signals.
//!
//! # v2.0 plan
//!
//! - Fork app processes; they inherit a MAP_SHARED backbuffer and a pipe pair.
//! - App signals SIGUSR1 when a frame is ready; bzdisplayd composites it to
//!   the framebuffer.

#![no_std]
#![no_main]

extern crate alloc;

mod framebuffer;
mod text_renderer;

use bazzulto_system::raw;
use bazzulto_display::font_manager::{FontManager, FontVariant};
use framebuffer::{FramebufferSurface, FramebufferError};
use text_renderer::{TextRenderer, Color};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    main();
    raw::raw_exit(0)
}

fn write_stdout(message: &str) {
    raw::raw_write(1, message.as_ptr(), message.len());
}

fn write_stderr(message: &str) {
    raw::raw_write(2, message.as_ptr(), message.len());
}

fn main() {
    // --- Step 1: map the framebuffer ---
    let mut surface = match FramebufferSurface::open() {
        Ok(surface) => surface,
        Err(FramebufferError::NotAvailable) => {
            write_stderr("bzdisplayd: no framebuffer available (headless?)\n");
            raw::raw_exit(1);
        }
        Err(FramebufferError::MappingFailed) => {
            write_stderr("bzdisplayd: sys_framebuffer_map failed\n");
            raw::raw_exit(1);
        }
    };

    // --- Step 2: initialise font manager ---
    let mut font_manager = FontManager::new();

    // Try to load the preferred console font.
    let font_id = match font_manager.load_font("JetBrainsMono", FontVariant::default()) {
        Ok(font_id) => font_id,
        Err(_) => {
            // No TTF available — use embedded fallback if compiled in.
            match font_manager.fallback_font_id() {
                Some(font_id) => font_id,
                None => {
                    write_stderr(
                        "bzdisplayd: no font available\
                         — place JetBrainsMono-Regular.ttf in /system/fonts/JetBrainsMono/\n",
                    );
                    // Stay resident; a future fontmanager install will populate
                    // /system/fonts/ and a service restart will find the font.
                    render_loop_no_font();
                }
            }
        }
    };

    // --- Step 3: render initial frame ---
    surface.clear();

    let mut renderer = TextRenderer::new(
        13.0,
        Color::WHITE,
        Color::BLACK,
        8, // left margin (pixels)
        8, // top margin (pixels)
        font_id,
        &mut font_manager,
    );

    // --- Step 4: main render loop ---
    //
    // Read text from stdin line by line and render each line to the
    // framebuffer.  bzinit wired the write ends of the display pipe to the
    // stdout/stderr of every service that has `display_output = true`, so
    // their output arrives here.
    //
    // Protocol: raw UTF-8 bytes.  '\n' triggers a new line on screen.
    // '\r' is ignored (some services may emit CRLF).
    render_loop(font_id, &mut font_manager, &mut surface, &mut renderer);
}

// ---------------------------------------------------------------------------
// Render loop
// ---------------------------------------------------------------------------

/// Read stdin and render UTF-8 text to the framebuffer.
///
/// Bytes are accumulated in a 4-byte buffer until a complete UTF-8 codepoint
/// is formed before passing it to the text renderer.  This correctly handles
/// multibyte sequences (2–4 bytes) such as accented characters, CJK ideographs,
/// and emoji.  Invalid sequences are discarded after 4 bytes have accumulated
/// without forming a valid codepoint — the same behavior as most terminals.
///
/// CR (`\r`) bytes are dropped; LF (`\n`) is rendered as a newline.
/// ANSI escape sequence parser state.
///
/// Recognises the subset of CSI (Control Sequence Introducer) sequences
/// used by Bazzulto coreutils:
///
///   ESC [ 2 J   — erase entire display (clear screen)
///   ESC [ H     — cursor home (move to row 1, col 1)
///   ESC [ K     — erase to end of line
///
/// Any unrecognised sequence is silently discarded once the final byte
/// (an ASCII letter) is received.
#[derive(Clone, Copy, PartialEq)]
enum AnsiState {
    /// Normal text rendering.
    Normal,
    /// Received ESC (0x1B), waiting for `[`.
    Escape,
    /// Inside a CSI sequence (`ESC [`), accumulating parameter bytes.
    Csi,
}

/// Set O_NONBLOCK on a file descriptor via fcntl(fd, F_SETFL, O_NONBLOCK).
fn set_nonblock(fd: i32) {
    const SYS_FCNTL: u64 = 128;
    const F_SETFL: u64 = 4;
    const O_NONBLOCK: u64 = 0x800;
    unsafe {
        core::arch::asm!(
            "svc #128",
            in("x0") fd as u64,
            in("x1") F_SETFL,
            in("x2") O_NONBLOCK,
            lateout("x0") _,
            options(nostack),
        );
    }
    let _ = SYS_FCNTL; // suppress unused warning
}

fn render_loop(
    font_id: bazzulto_display::font_manager::FontId,
    font_manager: &mut FontManager,
    surface: &mut framebuffer::FramebufferSurface,
    renderer: &mut text_renderer::TextRenderer,
) -> ! {
    let mut read_buf = [0u8; 256];

    // UTF-8 accumulator — holds a partial multibyte sequence between reads.
    let mut utf8_buf = [0u8; 4];
    let mut utf8_len: usize = 0;

    // ANSI escape sequence parser state.
    let mut ansi_state = AnsiState::Normal;
    let mut csi_buf = [0u8; 16];
    let mut csi_len: usize = 0;

    // Cursor blink state.
    let mut cursor_visible = true;
    let mut idle_ticks: u32 = 0;
    const BLINK_INTERVAL: u32 = 10; // 10 × 50ms = 500ms

    // Set stdin to non-blocking so we can blink the cursor while idle.
    set_nonblock(0);

    // Draw the initial cursor before entering the loop.
    renderer.draw_cursor_with_font(font_id, font_manager, surface);

    loop {
        let bytes_read = raw::raw_read(0, read_buf.as_mut_ptr(), read_buf.len());
        if bytes_read <= 0 {
            // No data available — blink the cursor while idle.
            idle_ticks += 1;
            if idle_ticks >= BLINK_INTERVAL {
                idle_ticks = 0;
                cursor_visible = !cursor_visible;
                if cursor_visible {
                    renderer.draw_cursor_with_font(font_id, font_manager, surface);
                } else {
                    renderer.erase_cursor_with_font(font_id, font_manager, surface);
                }
            }
            let sleep_ts: [u64; 2] = [0, 50_000_000]; // 50ms
            raw::raw_nanosleep(sleep_ts.as_ptr());
            continue;
        }

        // Data arrived — ensure cursor is visible and reset blink timer.
        idle_ticks = 0;
        if !cursor_visible {
            cursor_visible = true;
        }

        // Erase the cursor before updating the text so the cursor cell is
        // restored to its normal appearance before new glyphs are rendered.
        renderer.erase_cursor_with_font(font_id, font_manager, surface);

        for &byte in &read_buf[..bytes_read as usize] {
            // --- ANSI escape sequence state machine ---
            match ansi_state {
                AnsiState::Escape => {
                    if byte == b'[' {
                        ansi_state = AnsiState::Csi;
                        csi_len = 0;
                        continue;
                    }
                    // Not a CSI sequence — discard the ESC and process
                    // this byte as normal text.
                    ansi_state = AnsiState::Normal;
                    // fall through to Normal handling below
                }
                AnsiState::Csi => {
                    // CSI parameter bytes are 0x30–0x3F; intermediate 0x20–0x2F;
                    // final byte is 0x40–0x7E (an ASCII letter or symbol).
                    if byte >= 0x40 && byte <= 0x7E {
                        // Final byte — dispatch the sequence.
                        dispatch_csi(&csi_buf[..csi_len], byte, renderer, surface);
                        ansi_state = AnsiState::Normal;
                    } else if csi_len < csi_buf.len() {
                        csi_buf[csi_len] = byte;
                        csi_len += 1;
                    }
                    // else: overflow — keep consuming until final byte.
                    continue;
                }
                AnsiState::Normal => {}
            }

            // --- Normal text path ---

            // ESC byte starts a potential escape sequence.
            if byte == 0x1B {
                // Flush any partial UTF-8 sequence before entering escape mode.
                utf8_len = 0;
                ansi_state = AnsiState::Escape;
                continue;
            }

            // Drop CR; it appears in CRLF line endings and has no display role.
            if byte == b'\r' {
                continue;
            }

            // If we encounter an ASCII byte while holding a partial sequence,
            // the previous sequence was invalid — discard it and start fresh.
            if byte < 0x80 && utf8_len > 0 {
                utf8_len = 0;
            }

            utf8_buf[utf8_len] = byte;
            utf8_len += 1;

            // Attempt to decode the accumulated bytes as a UTF-8 string.
            match core::str::from_utf8(&utf8_buf[..utf8_len]) {
                Ok(text) => {
                    // Complete codepoint(s) decoded — render and reset buffer.
                    renderer.draw_str(text, font_id, font_manager, surface);
                    utf8_len = 0;
                }
                Err(_) if utf8_len == 4 => {
                    // Four bytes never formed a valid codepoint — discard.
                    utf8_len = 0;
                }
                Err(_) => {
                    // Partial sequence — keep accumulating.
                }
            }
        }

        // Redraw the cursor at the updated position after rendering is done.
        renderer.draw_cursor_with_font(font_id, font_manager, surface);
    }
}

/// Dispatch a completed CSI sequence.
///
/// `params` contains the parameter bytes between `ESC [` and the final byte.
/// `final_byte` is the command character (e.g. `J`, `H`, `K`).
/// Parse a CSI numeric parameter (e.g. "12" → 12). Returns 1 for empty params
/// (ANSI default for movement commands).
fn parse_csi_number(params: &[u8]) -> u32 {
    if params.is_empty() { return 1; }
    let mut value: u32 = 0;
    for &byte in params {
        if byte >= b'0' && byte <= b'9' {
            value = value.saturating_mul(10).saturating_add((byte - b'0') as u32);
        }
    }
    if value == 0 { 1 } else { value }
}

fn dispatch_csi(
    params: &[u8],
    final_byte: u8,
    renderer: &mut text_renderer::TextRenderer,
    surface: &mut framebuffer::FramebufferSurface,
) {
    match final_byte {
        // ESC[2J — erase entire display.
        b'J' if params == b"2" => {
            renderer.clear_screen(surface);
        }
        // ESC[H — cursor home (no parameters or "1;1").
        b'H' if params.is_empty() || params == b"1;1" => {
            renderer.cursor_home();
        }
        // ESC[K — erase from cursor to end of line.
        b'K' => {
            renderer.erase_to_end_of_line(surface);
        }
        // ESC[<n>C — cursor forward (right) by n columns.
        b'C' => {
            let count = parse_csi_number(params);
            renderer.cursor_forward(count, surface);
        }
        // ESC[<n>D — cursor backward (left) by n columns.
        b'D' => {
            let count = parse_csi_number(params);
            renderer.cursor_backward(count);
        }
        // Unrecognised — silently discard.
        _ => {}
    }
}

/// Called when no font is available.  Drains stdin so the pipe does not fill
/// up and block writers, but produces no visual output.
fn render_loop_no_font() -> ! {
    let mut buffer = [0u8; 256];
    loop {
        let result = raw::raw_read(0, buffer.as_mut_ptr(), buffer.len());
        if result <= 0 {
            raw::raw_yield();
        }
    }
}

// ---------------------------------------------------------------------------
// Panic / alloc-error handlers
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_write(2, b"bzdisplayd: panic\n".as_ptr(), 18);
    raw::raw_exit(1)
}

