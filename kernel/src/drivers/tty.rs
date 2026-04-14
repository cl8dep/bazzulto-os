// drivers/tty.rs — Simple TTY backed by the PL011 UART.
//
// Implements line-buffered (cooked) mode for normal use and raw mode for
// character-by-character reading.
//
// Signal delivery:
//   Ctrl+C (0x03) → SIGINT  (signal 2)  to foreground process.
//   Ctrl+Z (0x1A) → SIGTSTP (signal 20) to foreground process.
//   Ctrl+\ (0x1C) → SIGQUIT (signal 3)  to foreground process.
//
// Reference:
//   POSIX.1-2017 §11 (General Terminal Interface).
//   Linux tty/n_tty.c (line discipline reference).

use core::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// POSIX signal numbers used here
// ---------------------------------------------------------------------------

/// SIGINT — interactive attention signal.
const SIGINT: u8 = 2;

/// SIGQUIT — quit signal (Ctrl+\).
const SIGQUIT: u8 = 3;

/// SIGTSTP — terminal stop signal (Ctrl+Z).
const SIGTSTP: u8 = 20;

// ---------------------------------------------------------------------------
// POSIX termios flag constants (c_lflag bits)
// ---------------------------------------------------------------------------

/// ICANON — enable canonical (line-buffered) mode.
pub const TERMIOS_ICANON: u32 = 0x0002;

/// ECHO — enable echo of input characters.
pub const TERMIOS_ECHO: u32 = 0x0008;

// ---------------------------------------------------------------------------
// POSIX termios structure
// ---------------------------------------------------------------------------

/// Subset of POSIX `struct termios`.
///
/// Only the fields relevant to the current TTY implementation are used.
/// Reference: POSIX.1-2017 §11.1.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Termios {
    /// Input mode flags (not yet interpreted by this driver).
    pub c_iflag: u32,
    /// Output mode flags (not yet interpreted by this driver).
    pub c_oflag: u32,
    /// Control mode flags (not yet interpreted by this driver).
    pub c_cflag: u32,
    /// Local mode flags.  ICANON and ECHO are the two active ones.
    pub c_lflag: u32,
    /// Control characters array (POSIX NCCS = 32).
    pub c_cc: [u8; 32],
}

impl Termios {
    /// Default cooked-mode termios: ICANON | ECHO set.
    pub const fn cooked_defaults() -> Self {
        Self {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: TERMIOS_ICANON | TERMIOS_ECHO,
            c_cc: [0u8; 32],
        }
    }
}

// ---------------------------------------------------------------------------
// TTY modes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TtyMode {
    /// Line-buffered mode.  Input is delivered to the process after Enter.
    Cooked,
    /// Character-by-character mode.  No line buffering, no echo processing.
    Raw,
}

// ---------------------------------------------------------------------------
// TTY line buffer
// ---------------------------------------------------------------------------

/// Maximum characters in the cooked-mode line buffer.
///
/// POSIX MAX_CANON: at least 255.  We use 4096 to match Linux's N_TTY_BUF_SIZE.
const TTY_LINE_BUFFER_CAPACITY: usize = 4096;

// ---------------------------------------------------------------------------
// Window size globals
// ---------------------------------------------------------------------------

/// Terminal row count, set by the console driver after framebuffer init.
static mut TERMINAL_ROWS: u16 = 25;

/// Terminal column count, set by the console driver after framebuffer init.
static mut TERMINAL_COLS: u16 = 80;

// ---------------------------------------------------------------------------
// TTY state
// ---------------------------------------------------------------------------

struct TtyState {
    mode: TtyMode,
    termios: Termios,
    line_buffer: [u8; TTY_LINE_BUFFER_CAPACITY],
    line_buffer_length: usize,
    /// True when a complete line (terminated by '\n') is available in the buffer.
    line_ready: bool,
    /// Optional pipe buffer to receive echo output in addition to UART.
    ///
    /// When non-null, `cooked_mode_process_byte` writes echoed characters here
    /// so they appear on the graphical display (bzdisplayd) as well as serial.
    /// Set by `tty_set_echo_sink` before a TTY read; cleared after.
    ///
    /// SAFETY: the pointer is valid for the duration of the TTY read syscall
    /// (IRQs disabled, same execution context). Cleared before the caller's fd
    /// table lock is released, so the pipe buffer cannot be freed beneath us.
    echo_pipe_buffer: *mut crate::fs::pipe::PipeBuffer,
}

impl TtyState {
    const fn new() -> Self {
        Self {
            mode: TtyMode::Cooked,
            termios: Termios::cooked_defaults(),
            line_buffer: [0u8; TTY_LINE_BUFFER_CAPACITY],
            line_buffer_length: 0,
            line_ready: false,
            echo_pipe_buffer: core::ptr::null_mut(),
        }
    }
}

struct SyncTtyState(UnsafeCell<TtyState>);
// SAFETY: single-core; IRQs must be disabled when calling TTY functions.
unsafe impl Sync for SyncTtyState {}

static TTY: SyncTtyState = SyncTtyState(UnsafeCell::new(TtyState::new()));

// ---------------------------------------------------------------------------
// PL011 receive register — same physical address as the UART TX.
// ---------------------------------------------------------------------------

/// Read one character from the UART receive FIFO, blocking until available.
///
/// Reads the UARTFR.RXFE flag (bit 4) to wait for data.
/// Reference: PL011 TRM DDI 0183G §3.3.3.
pub fn uart_receive_blocking() -> u8 {
    // FR register offset 0x018; RXFE = bit 4.
    // DR register offset 0x000.
    unsafe {
        let base = uart_base();
        if base == 0 {
            return 0;
        }
        // Wait until RX FIFO is not empty (RXFE = 0).
        // Yield to the scheduler on each spin so other processes (e.g.
        // bzdisplayd) can run while this process waits for keyboard input.
        loop {
            let fr = core::ptr::read_volatile((base + 0x018) as *const u32);
            if fr & (1 << 4) == 0 {
                break;
            }
            // FIFO empty — let other processes run before polling again.
            crate::scheduler::schedule_next();
        }
        let dr = core::ptr::read_volatile(base as *const u32);
        (dr & 0xFF) as u8
    }
}

/// Non-blocking check: returns Some(byte) if a character is waiting.
pub fn uart_receive_nonblocking() -> Option<u8> {
    unsafe {
        let base = uart_base();
        if base == 0 {
            return None;
        }
        let fr = core::ptr::read_volatile((base + 0x018) as *const u32);
        if fr & (1 << 4) != 0 {
            return None; // RXFE set — FIFO empty
        }
        let dr = core::ptr::read_volatile(base as *const u32);
        Some((dr & 0xFF) as u8)
    }
}

/// UART base address — set by the UART driver's early_init.
///
/// Reads the same global that uart.rs maintains.  The TTY and UART share the
/// same physical device.
fn uart_base() -> usize {
    // Access the UART_BASE static from uart.rs via the raw symbol.
    // We rely on the fact that uart.rs exposes it as `static mut UART_BASE`.
    // Use the same pattern: call through a known function rather than accessing
    // the private static directly.  We expose a helper in uart.rs for this.
    crate::drivers::uart::uart_base_address()
}

// ---------------------------------------------------------------------------
// TTY read — called from the VFS layer
// ---------------------------------------------------------------------------

/// Read up to `destination.len()` bytes from the TTY, applying the current mode.
///
/// In cooked mode: blocks until a full line is entered, then delivers up to
/// `destination.len()` bytes.  The '\n' is included.
///
/// In raw mode: reads exactly one character and returns it immediately.
///
/// # Safety
/// Must be called with IRQs disabled (for scheduler interactions).
pub unsafe fn tty_read_bytes(destination: &mut [u8]) -> usize {
    if destination.is_empty() {
        return 0;
    }

    let state = &mut *TTY.0.get();

    match state.mode {
        TtyMode::Raw => {
            let byte = uart_receive_blocking();
            destination[0] = byte;
            1
        }
        TtyMode::Cooked => {
            // Fill the line buffer until a newline or buffer-full condition.
            //
            // We use a nonblocking UART poll + scheduler yield instead of
            // uart_receive_blocking so that:
            //   1. bzdisplayd and other processes can run while we wait.
            //   2. Virtio keyboard IRQs can fire (at EL0 of the yielded-to
            //      process) and inject bytes via tty_receive_char(), setting
            //      line_ready before we return here.
            loop {
                if state.line_ready {
                    break;
                }
                if let Some(byte) = uart_receive_nonblocking() {
                    cooked_mode_process_byte(state, byte);
                } else {
                    // No UART byte — yield so other processes run and IRQs
                    // (including the virtio keyboard IRQ) can be delivered.
                    //
                    // After yielding, briefly enable IRQs and execute WFI so
                    // that the keyboard IRQ can fire even when bzsh is the only
                    // runnable process (the scheduler returns immediately in that
                    // case, leaving DAIF.I=1 — without WFI the IRQ would never
                    // be taken and the shell would spin forever).
                    //
                    // ARM guarantee: if an unmasked IRQ is pending at the WFI
                    // instruction, WFI returns immediately — no lost events.
                    //
                    // Reference: ARM ARM DDI 0487, §D1.17.2 "WFI".
                    crate::scheduler::schedule_next();
                    unsafe {
                        // Enable IRQs so the keyboard IRQ can be taken at EL1.
                        core::arch::asm!("msr daifclr, #2", options(nostack, nomem));
                        // WFI — no `nomem` so the compiler treats this as a full
                        // memory barrier.  Without this, the compiler may cache
                        // state.line_ready in a register and never re-read it from
                        // RAM after the IRQ handler modifies it.
                        // Reference: ARM ARM DDI 0487, §D1.17.2 "WFI".
                        core::arch::asm!("wfi");
                        // Disable IRQs before continuing kernel code.
                        core::arch::asm!("msr daifset, #2", options(nostack, nomem));
                    }
                    // Explicit compiler fence to ensure state.line_ready is
                    // re-read from memory on the next loop iteration.
                    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
                }
            }

            // Copy the line into the destination.
            let to_copy = destination.len().min(state.line_buffer_length);
            destination[..to_copy].copy_from_slice(&state.line_buffer[..to_copy]);

            // Shift remaining bytes left (partial reads).
            let remaining = state.line_buffer_length - to_copy;
            if remaining > 0 {
                state.line_buffer.copy_within(to_copy..state.line_buffer_length, 0);
            }
            state.line_buffer_length = remaining;
            if remaining == 0 {
                state.line_ready = false;
            }
            to_copy
        }
    }
}

/// Process one byte in cooked mode.
///
/// Handles:
///   - '\r' (0x0D): convert to '\n' and mark line ready.
///   - '\n' (0x0A): append to buffer and mark line ready.
///   - BS / DEL (0x08 / 0x7F): erase last character with VT100 backspace sequence.
///   - ETX (0x03): send SIGINT to the foreground process.
///   - SUB (0x1A): send SIGTSTP to the foreground process.
///   - FS  (0x1C): send SIGQUIT to the foreground process.
///   - All other printable bytes: append to buffer and echo.
fn cooked_mode_process_byte(state: &mut TtyState, byte: u8) {
    match byte {
        0x03 => {
            // Ctrl+C: deliver SIGINT to the foreground process.
            unsafe { deliver_signal_to_foreground(SIGINT) };
            crate::drivers::uart::puts("^C\r\n");
        }
        0x1A => {
            // Ctrl+Z: deliver SIGTSTP to the foreground process.
            unsafe { deliver_signal_to_foreground(SIGTSTP) };
            crate::drivers::uart::puts("^Z\r\n");
        }
        0x1C => {
            // Ctrl+\: deliver SIGQUIT to the foreground process.
            unsafe { deliver_signal_to_foreground(SIGQUIT) };
            crate::drivers::uart::puts("^\\\r\n");
        }
        0x08 | 0x7F => {
            // Backspace / DEL: erase last character.
            if state.line_buffer_length > 0 {
                state.line_buffer_length -= 1;
                tty_echo_bytes(state, b"\x08 \x08"); // VT100 backspace-space-backspace
            }
        }
        b'\r' | b'\n' => {
            if state.line_buffer_length < TTY_LINE_BUFFER_CAPACITY {
                state.line_buffer[state.line_buffer_length] = b'\n';
                state.line_buffer_length += 1;
            }
            state.line_ready = true;
            tty_echo_bytes(state, b"\r\n");
        }
        printable if printable >= 0x20 && printable < 0x7F => {
            if state.line_buffer_length < TTY_LINE_BUFFER_CAPACITY {
                state.line_buffer[state.line_buffer_length] = printable;
                state.line_buffer_length += 1;
                // Echo the character to UART and display pipe.
                tty_echo_bytes(state, &[printable]);
            }
        }
        _ => {} // ignore other control characters
    }
}

/// Write echo bytes to UART and, if set, to the display pipe buffer.
///
/// Called from `cooked_mode_process_byte` for every echoed character.
/// Writing directly to the pipe buffer avoids the scheduler and is safe
/// here because IRQs are disabled (single-core invariant).
fn tty_echo_bytes(state: &TtyState, bytes: &[u8]) {
    // Always echo to UART / serial.
    for &b in bytes {
        crate::drivers::uart::putc(b);
    }
    // Also write to the display pipe if one is registered.
    if !state.echo_pipe_buffer.is_null() {
        unsafe {
            let buf = &mut *state.echo_pipe_buffer;
            buf.write_bytes(bytes);
            // Wake a blocked reader (bzdisplayd) if there is one.
            buf.wake_blocked_reader();
        }
    }
}

// ---------------------------------------------------------------------------
// Signal delivery
// ---------------------------------------------------------------------------

/// Send the given signal to the foreground process group.
///
/// Reads `TERMINAL_FOREGROUND_PGID` and delivers the signal to all processes
/// in that group via `Scheduler::send_signal_to_group`.  If no foreground group
/// is set (pgid == 0), falls back to delivering to any process marked
/// `is_foreground` (legacy path).
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn deliver_signal_to_foreground(signal: u8) {
    let pgid = crate::systemcalls::terminal_foreground_pgid();
    crate::scheduler::with_scheduler(|scheduler| {
        if pgid != 0 {
            scheduler.send_signal_to_group(pgid, signal);
        } else {
            // Legacy fallback: no foreground pgid set — signal any foreground process.
            for slot_index in 0..32768usize {
                if let Some(process) = scheduler.process(crate::process::Pid::new(slot_index as u16, 1)) {
                    if process.is_foreground {
                        let foreground_pid = process.pid;
                        scheduler.send_signal_to(foreground_pid, signal);
                        return;
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Echo sink — lets sys_read route cooked-mode echo to the display pipe
// ---------------------------------------------------------------------------

/// Register a pipe buffer as the echo sink for the duration of a TTY read.
///
/// Called by `sys_read` immediately before blocking in cooked mode.  The
/// pointer must remain valid until `tty_clear_echo_sink` is called.
///
/// # Safety
/// Must be called with IRQs disabled.  `buf` must outlive the TTY read.
pub unsafe fn tty_set_echo_sink(buf: *mut crate::fs::pipe::PipeBuffer) {
    let state = &mut *TTY.0.get();
    state.echo_pipe_buffer = buf;
}

/// Clear the echo sink registered by `tty_set_echo_sink`.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn tty_clear_echo_sink() {
    let state = &mut *TTY.0.get();
    state.echo_pipe_buffer = core::ptr::null_mut();
}

// ---------------------------------------------------------------------------
// tty_receive_char — inject a character from a keyboard driver
// ---------------------------------------------------------------------------

/// Receive one character from a hardware keyboard driver (IRQ context).
///
/// Processes the character through cooked-mode logic (same as UART input).
/// Called from keyboard_virtio::keyboard_irq_handler.
///
/// # Safety
/// Must be called from an IRQ handler with IRQs masked at EL1.
pub unsafe fn tty_receive_char(byte: u8) {
    let state = &mut *TTY.0.get();
    cooked_mode_process_byte(state, byte);
}

// ---------------------------------------------------------------------------
// Mode control
// ---------------------------------------------------------------------------

/// Switch the TTY to raw mode.
pub fn tty_set_raw_mode() {
    unsafe {
        let state = &mut *TTY.0.get();
        state.mode = TtyMode::Raw;
        state.termios.c_lflag &= !TERMIOS_ICANON;
    }
}

/// Switch the TTY to cooked (line-buffered) mode.
pub fn tty_set_cooked_mode() {
    unsafe {
        let state = &mut *TTY.0.get();
        state.mode = TtyMode::Cooked;
        state.termios.c_lflag |= TERMIOS_ICANON;
    }
}

// ---------------------------------------------------------------------------
// termios get/set
// ---------------------------------------------------------------------------

/// Copy the current termios settings into `*termios_ptr`.
///
/// # Safety
/// `termios_ptr` must be a valid, writable pointer to a `Termios`.
/// Must be called with IRQs disabled.
pub unsafe fn tty_tcgetattr(termios_ptr: *mut Termios) {
    let state = &*TTY.0.get();
    core::ptr::write(termios_ptr, state.termios);
}

/// Apply new termios settings from `*termios_ptr`.
///
/// If `c_lflag & ICANON == 0` the TTY switches to raw mode; otherwise cooked.
///
/// # Safety
/// `termios_ptr` must be a valid, readable pointer to a `Termios`.
/// Must be called with IRQs disabled.
pub unsafe fn tty_tcsetattr(termios_ptr: *const Termios) {
    let new_termios = core::ptr::read(termios_ptr);
    let state = &mut *TTY.0.get();
    state.termios = new_termios;
    if new_termios.c_lflag & TERMIOS_ICANON == 0 {
        state.mode = TtyMode::Raw;
    } else {
        state.mode = TtyMode::Cooked;
    }
}

// ---------------------------------------------------------------------------
// Window size
// ---------------------------------------------------------------------------

/// Record the terminal dimensions (called from console.rs after framebuffer init).
pub fn tty_set_winsize(rows: u16, cols: u16) {
    unsafe {
        TERMINAL_ROWS = rows;
        TERMINAL_COLS = cols;
    }
}

/// Write the terminal dimensions into the caller's variables.
///
/// # Safety
/// Both pointers must be valid and writable.
pub unsafe fn tty_get_winsize(rows: *mut u16, cols: *mut u16) {
    core::ptr::write(rows, TERMINAL_ROWS);
    core::ptr::write(cols, TERMINAL_COLS);
}

/// Return the terminal dimensions as a `(rows, cols)` tuple.
pub fn tty_get_winsize_pair() -> (u16, u16) {
    unsafe { (TERMINAL_ROWS, TERMINAL_COLS) }
}
