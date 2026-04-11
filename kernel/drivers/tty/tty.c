#include "../../../include/bazzulto/tty.h"
#include "../../../include/bazzulto/console.h"
#include "../../../include/bazzulto/hal/hal_uart.h"
#include "../../../include/bazzulto/waitqueue.h"
#include "../../../include/bazzulto/scheduler.h"

// ---------------------------------------------------------------------------
// TTY line discipline
//
// In cooked mode:
//   - Characters are accumulated in line_buffer until Enter.
//   - Backspace removes the last character and erases the echo.
//   - Ctrl+C sends SIGINT to the foreground process.
//   - Ctrl+D signals EOF (if line is empty) or flushes the buffer.
//   - Ctrl+Z sends SIGTSTP (suspend).
//   - Ctrl+L clears the screen.
//   - All printable characters are echoed to the console.
//   - When Enter is pressed, the line (including '\n') is copied to the
//     output buffer where tty_read() can consume it.
//
// In raw mode:
//   - Every byte goes directly to the output buffer.
//   - No echo, no line editing, no signal generation.
// ---------------------------------------------------------------------------

// Control character constants
#define CTRL_C   0x03   // ETX — interrupt (SIGINT)
#define CTRL_D   0x04   // EOT — end of file
#define CTRL_L   0x0C   // FF  — clear screen
#define CTRL_Z   0x1A   // SUB — suspend (SIGTSTP)

// Signal numbers (must match include/bazzulto/signal.h)
#define SIGNAL_SIGINT   2
#define SIGNAL_SIGTSTP  20

// Line editing buffer — where the user types in cooked mode.
static char     line_buffer[TTY_LINE_BUFFER_SIZE];
static int      line_position;

// Output buffer — completed lines (cooked) or raw bytes ready for tty_read().
// This is a ring buffer consumed by tty_read().
#define TTY_RING_SIZE 2048
static char          output_ring[TTY_RING_SIZE];
static volatile uint32_t ring_head;   // writer (ISR/cooked flush)
static volatile uint32_t ring_tail;   // reader (tty_read)

// Wait queue — tty_read() sleeps here when the output ring is empty.
static wait_queue_t  tty_wait_queue = WAIT_QUEUE_INIT;

// Mode and echo state.
static int tty_mode;
static int tty_echo_enabled;
static int tty_eof_pending;  // set by Ctrl+D on empty line

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

static void ring_put(char c)
{
    uint32_t next = (ring_head + 1) % TTY_RING_SIZE;
    if (next == ring_tail)
        return;  // full — drop
    output_ring[ring_head] = c;
    ring_head = next;
}

static void echo_char(char c)
{
    if (!tty_echo_enabled)
        return;
    console_putc(c);
    hal_uart_putc(c);
}

static void echo_control(char letter)
{
    // Echo control chars as ^X (e.g. ^C, ^D, ^Z)
    if (!tty_echo_enabled)
        return;
    console_putc('^');
    console_putc(letter);
    hal_uart_putc('^');
    hal_uart_putc(letter);
}

static void flush_line_to_ring(void)
{
    for (int i = 0; i < line_position; i++)
        ring_put(line_buffer[i]);
    ring_put('\n');
    line_position = 0;
    process_wakeup(&tty_wait_queue);
}

// ---------------------------------------------------------------------------
// Cooked mode character processing
// ---------------------------------------------------------------------------

static void tty_receive_cooked(char c)
{
    // Ctrl+C → SIGINT
    if (c == CTRL_C) {
        echo_control('C');
        echo_char('\n');
        line_position = 0;  // discard current line
        scheduler_send_signal_to_foreground(SIGNAL_SIGINT);
        return;
    }

    // Ctrl+Z → SIGTSTP
    if (c == CTRL_Z) {
        echo_control('Z');
        echo_char('\n');
        scheduler_send_signal_to_foreground(SIGNAL_SIGTSTP);
        return;
    }

    // Ctrl+D → EOF or flush
    if (c == CTRL_D) {
        if (line_position == 0) {
            // Empty line — signal EOF to reader
            tty_eof_pending = 1;
            process_wakeup(&tty_wait_queue);
        } else {
            // Non-empty — flush what we have without adding '\n'
            for (int i = 0; i < line_position; i++)
                ring_put(line_buffer[i]);
            line_position = 0;
            process_wakeup(&tty_wait_queue);
        }
        return;
    }

    // Ctrl+L → clear screen
    if (c == CTRL_L) {
        console_clear();
        // Re-echo the current line
        for (int i = 0; i < line_position; i++)
            echo_char(line_buffer[i]);
        return;
    }

    // Backspace (0x7F or 0x08)
    if (c == 0x7F || c == 0x08) {
        if (line_position > 0) {
            line_position--;
            // Erase the character from the screen: "\b \b"
            echo_char('\b');
            echo_char(' ');
            echo_char('\b');
        }
        return;
    }

    // Enter / Return
    if (c == '\r' || c == '\n') {
        echo_char('\n');
        flush_line_to_ring();
        return;
    }

    // Regular printable character — add to line buffer
    if (line_position < TTY_LINE_BUFFER_SIZE - 1) {
        line_buffer[line_position++] = c;
        echo_char(c);
    }
    // If buffer is full, silently drop (user should press Enter)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

void tty_init(void)
{
    line_position    = 0;
    ring_head        = 0;
    ring_tail        = 0;
    tty_mode         = TTY_MODE_COOKED;
    tty_echo_enabled = 1;
    tty_eof_pending  = 0;
}

void tty_receive_char(char character)
{
    if (tty_mode == TTY_MODE_RAW) {
        // Raw mode: pass through immediately, no processing
        ring_put(character);
        process_wakeup(&tty_wait_queue);
        return;
    }

    tty_receive_cooked(character);
}

int64_t tty_read(char *buf, size_t len)
{
    if (len == 0)
        return 0;

    // Disable IRQs to prevent race between checking ring and sleeping.
    __asm__ volatile("msr daifset, #2");

    while (ring_head == ring_tail && !tty_eof_pending) {
        // Check for pending signals.
        process_t *proc = scheduler_get_current();
        if (proc && proc->pending_signals) {
            __asm__ volatile("msr daifclr, #2");
            return -1;  // interrupted
        }
        process_sleep(&tty_wait_queue);
        __asm__ volatile("msr daifset, #2");
    }

    // Check EOF
    if (ring_head == ring_tail && tty_eof_pending) {
        tty_eof_pending = 0;
        __asm__ volatile("msr daifclr, #2");
        return 0;  // EOF
    }

    // Copy available bytes from the ring.
    size_t count = 0;
    while (count < len && ring_tail != ring_head) {
        buf[count++] = output_ring[ring_tail];
        ring_tail = (ring_tail + 1) % TTY_RING_SIZE;

        // In cooked mode, stop after delivering one line (after '\n').
        if (tty_mode == TTY_MODE_COOKED && buf[count - 1] == '\n')
            break;
    }

    __asm__ volatile("msr daifclr, #2");
    return (int64_t)count;
}

void tty_set_mode(int mode)
{
    tty_mode = mode;
    if (mode == TTY_MODE_RAW) {
        tty_echo_enabled = 0;
        line_position = 0;  // discard partial cooked line
    } else {
        tty_echo_enabled = 1;
    }
}

int tty_get_mode(void)
{
    return tty_mode;
}

void tty_set_echo(int enabled)
{
    tty_echo_enabled = enabled;
}
