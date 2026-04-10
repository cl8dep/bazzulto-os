#pragma once

#include <stdint.h>

// Initialize the PL011 UART at 115200 8N1.
// Must be called after virtual memory is active and the UART MMIO page
// is mapped at hhdm_offset + 0x09000000.
void uart_init(void);

// Transmit a single character (blocks until TX FIFO has space).
void uart_putc(char c);

// Receive a single character (blocks until a character is available).
// In the polling implementation, this busy-waits on the RX FIFO.
// After Phase 1 Step 4, this will block via wait queue + IRQ.
char uart_getc(void);

// Transmit a null-terminated string. Emits \r before each \n for
// terminal compatibility.
void uart_puts(const char *str);

// Returns 1 if there is at least one character in the RX FIFO, 0 otherwise.
int uart_rx_ready(void);

// Called from the IRQ dispatcher when INTID 33 fires.
// Drains the RX FIFO into a ring buffer and wakes blocked readers.
void uart_irq_handler(void);
