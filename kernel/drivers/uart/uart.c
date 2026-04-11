#include "../../../include/bazzulto/uart.h"
#include "../../../include/bazzulto/gic.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/waitqueue.h"
#include "../../../include/bazzulto/input.h"

// ---------------------------------------------------------------------------
// PL011 UART register map — ARM DDI 0183G (PL011 Technical Reference Manual)
//
// On QEMU virt, the first PL011 is at physical 0x09000000.
// We access it through the HHDM: hhdm_offset + 0x09000000.
// ---------------------------------------------------------------------------

#define UART_BASE (hhdm_offset + 0x09000000ULL)

// DDI 0183G Section 3.3 — register offsets
#define UARTDR    (*(volatile uint32_t *)(UART_BASE + 0x000)) // Data Register
#define UARTFR    (*(volatile uint32_t *)(UART_BASE + 0x018)) // Flag Register
#define UARTIBRD  (*(volatile uint32_t *)(UART_BASE + 0x024)) // Integer Baud Rate
#define UARTFBRD  (*(volatile uint32_t *)(UART_BASE + 0x028)) // Fractional Baud Rate
#define UARTLCR_H (*(volatile uint32_t *)(UART_BASE + 0x02C)) // Line Control
#define UARTCR    (*(volatile uint32_t *)(UART_BASE + 0x030)) // Control Register
#define UARTIMSC  (*(volatile uint32_t *)(UART_BASE + 0x038)) // Interrupt Mask Set/Clear
#define UARTICR   (*(volatile uint32_t *)(UART_BASE + 0x044)) // Interrupt Clear

// UARTFR bits — DDI 0183G Section 3.3.3
#define FR_TXFF  (1 << 5)  // TX FIFO full
#define FR_RXFE  (1 << 4)  // RX FIFO empty
#define FR_BUSY  (1 << 3)  // UART busy transmitting

// UARTLCR_H bits — DDI 0183G Section 3.3.7
#define LCR_WLEN_8BIT (0b11 << 5) // 8-bit word length
#define LCR_FEN       (1 << 4)    // FIFO enable

// UARTCR bits — DDI 0183G Section 3.3.8
#define CR_UARTEN (1 << 0)  // UART enable
#define CR_TXE    (1 << 8)  // Transmit enable
#define CR_RXE    (1 << 9)  // Receive enable

// UARTIMSC bits — DDI 0183G Section 3.3.10
#define IMSC_RXIM (1 << 4)  // Receive interrupt mask
#define IMSC_RTIM (1 << 6)  // Receive timeout interrupt mask

// ---------------------------------------------------------------------------
// RX ring buffer — single-producer (IRQ handler) / single-consumer (reader)
// ---------------------------------------------------------------------------
#define RX_BUF_SIZE 64

static char rx_buf[RX_BUF_SIZE];
static volatile uint32_t rx_head;  // ISR writes here
static volatile uint32_t rx_tail;  // reader reads here
static wait_queue_t rx_wq = WAIT_QUEUE_INIT;

void uart_init(void) {
	// DDI 0183G Section 3.3.8: disable UART before reconfiguration.
	UARTCR = 0;

	// Wait for any in-progress transmission to complete.
	while (UARTFR & FR_BUSY)
		;

	// Set baud rate to 115200.
	// QEMU virt PL011 uses a 24 MHz reference clock.
	// Divisor = 24000000 / (16 * 115200) = 13.0208...
	// IBRD = 13, FBRD = round(0.0208 * 64 + 0.5) = 1
	UARTIBRD = 13;
	UARTFBRD = 1;

	// 8 data bits, no parity, 1 stop bit, FIFOs enabled.
	UARTLCR_H = LCR_WLEN_8BIT | LCR_FEN;

	// Clear all pending interrupts.
	UARTICR = 0x7FF;

	// Enable UART, TX, and RX.
	UARTCR = CR_UARTEN | CR_TXE | CR_RXE;

	// Configure GIC for UART RX interrupt (INTID 33, SPI 1).
	gic_enable_spi(IRQ_UART0);

	// Unmask RX and Receive Timeout interrupts — DDI 0183G §3.3.10.
	// RXIM fires when the RX FIFO reaches the trigger level (default 2 chars).
	// RTIM fires when chars sit in the FIFO without new data arriving
	// (32 bit-period timeout). Together they ensure single-char responsiveness.
	UARTIMSC = IMSC_RXIM | IMSC_RTIM;
}

void uart_putc(char c) {
	// Spin while the TX FIFO is full — DDI 0183G Section 3.3.3.
	while (UARTFR & FR_TXFF)
		;
	UARTDR = (uint32_t)c & 0xFF;
}

char uart_getc(void) {
	// Disable IRQs to prevent a race between checking the buffer
	// and going to sleep.
	__asm__ volatile("msr daifset, #2");

	while (rx_head == rx_tail) {
		// Buffer empty — block until the IRQ handler puts data in.
		process_sleep(&rx_wq);
		// process_sleep yields and returns when we are woken up.
		// Re-disable IRQs for the next loop check.
		__asm__ volatile("msr daifset, #2");
	}

	char c = rx_buf[rx_tail];
	rx_tail = (rx_tail + 1) % RX_BUF_SIZE;

	__asm__ volatile("msr daifclr, #2");
	return c;
}

int uart_rx_ready(void) {
	return !(UARTFR & FR_RXFE);
}

void uart_puts(const char *str) {
	for (; *str; str++) {
		if (*str == '\n')
			uart_putc('\r');
		uart_putc(*str);
	}
}

void uart_irq_handler(void) {
	// Drain all available characters from the RX FIFO.
	while (!(UARTFR & FR_RXFE)) {
		char c = (char)(UARTDR & 0xFF);
		uint32_t next_head = (rx_head + 1) % RX_BUF_SIZE;
		if (next_head != rx_tail) {  // drop if buffer full
			rx_buf[rx_head] = c;
			rx_head = next_head;
		}
		// Feed the character to the input layer so stdin consumers (VFS fd 0)
		// receive serial input regardless of whether a keyboard is also present.
		input_emit_char(c);
	}

	// Clear both RX and timeout interrupts AFTER draining the FIFO.
	// Clearing before draining could lose a character that arrives
	// between the clear and the read — DDI 0183G Section 3.3.13.
	UARTICR = IMSC_RXIM | IMSC_RTIM;

	// Wake one blocked reader (if any).
	process_wakeup(&rx_wq);
}
