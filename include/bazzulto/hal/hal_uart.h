#pragma once

// ---------------------------------------------------------------------------
// HAL: Serial Console (UART)
//
// Platform-independent interface for serial I/O.
// The platform backend (e.g. PL011 for QEMU virt) implements these functions.
// ---------------------------------------------------------------------------

// Initialize the UART hardware (baud rate, FIFOs, interrupt enable).
void hal_uart_init(void);

// Transmit a single character. Busy-waits if the TX FIFO is full.
void hal_uart_putc(char c);

// Receive a single character. Blocks until data is available.
char hal_uart_getc(void);

// Transmit a null-terminated string. Inserts CR before each LF.
void hal_uart_puts(const char *str);

// Handle the UART RX interrupt. Drains the RX FIFO and feeds characters
// to the input layer via input_emit_char().
void hal_uart_irq_handler(void);
