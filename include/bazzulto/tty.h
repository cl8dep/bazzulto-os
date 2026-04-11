#pragma once

#include <stdint.h>
#include <stddef.h>

// ---------------------------------------------------------------------------
// TTY (Teletype) — line discipline layer
//
// The TTY sits between the keyboard driver and the process reading stdin.
// It provides two modes:
//
//   COOKED (default): the TTY accumulates characters into a line buffer,
//   handles backspace, echo, and control characters (Ctrl+C → SIGINT,
//   Ctrl+D → EOF, Ctrl+Z → SIGTSTP). The process only receives complete
//   lines after the user presses Enter.
//
//   RAW: every byte from the keyboard is delivered immediately to the
//   process without echo, buffering, or interpretation of control keys.
//   Used by editors, games, and anything that needs per-keypress input.
//
// The keyboard driver calls tty_receive_char() for each byte.
// The VFS stdin read path calls tty_read() to consume buffered input.
// ---------------------------------------------------------------------------

#define TTY_LINE_BUFFER_SIZE   1024
#define TTY_OUTPUT_BUFFER_SIZE 256

#define TTY_MODE_COOKED  0
#define TTY_MODE_RAW     1

// Initialize the TTY subsystem. Must be called after input_init() and
// before any keyboard input is expected.
void tty_init(void);

// Called by the keyboard driver (from IRQ context) for each translated
// character. In cooked mode, the TTY processes control keys, handles
// echo, and buffers the line. In raw mode, it passes the byte through
// immediately.
void tty_receive_char(char character);

// Called by the VFS stdin read path. Blocks until at least one byte is
// available, then copies up to `len` bytes into `buf`.
// Returns the number of bytes read, or -1 if interrupted by a signal.
// In cooked mode, returns one complete line (up to `len` bytes).
// In raw mode, returns whatever bytes are available.
int64_t tty_read(char *buf, size_t len);

// Switch the TTY between cooked and raw mode.
// In raw mode, echo is disabled and no line editing occurs.
void tty_set_mode(int mode);

// Get the current TTY mode (TTY_MODE_COOKED or TTY_MODE_RAW).
int tty_get_mode(void);

// Enable or disable character echo. In cooked mode, echo is on by default.
// In raw mode, echo is off by default but can be re-enabled.
void tty_set_echo(int enabled);
