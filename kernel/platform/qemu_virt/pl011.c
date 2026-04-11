// PL011 UART backend for QEMU virt — ARM DDI 0183G
//
// QEMU virt maps the first PL011 at physical 0x09000000.
// Accessed through the HHDM: hhdm_offset + 0x09000000.

#include "../../../include/bazzulto/hal/hal_uart.h"
#include "../../../include/bazzulto/hal/hal_irq.h"
#include "../../../include/bazzulto/kernel.h"
#include "../../../include/bazzulto/waitqueue.h"
#include "../../../include/bazzulto/tty.h"

// ---------------------------------------------------------------------------
// PL011 register map — ARM DDI 0183G (PL011 Technical Reference Manual)
// ---------------------------------------------------------------------------

#define UART_BASE (hhdm_offset + 0x09000000ULL)

// DDI 0183G Section 3.3 — register offsets
#define UARTDR    (*(volatile uint32_t *)(UART_BASE + 0x000))
#define UARTFR    (*(volatile uint32_t *)(UART_BASE + 0x018))
#define UARTIBRD  (*(volatile uint32_t *)(UART_BASE + 0x024))
#define UARTFBRD  (*(volatile uint32_t *)(UART_BASE + 0x028))
#define UARTLCR_H (*(volatile uint32_t *)(UART_BASE + 0x02C))
#define UARTCR    (*(volatile uint32_t *)(UART_BASE + 0x030))
#define UARTIMSC  (*(volatile uint32_t *)(UART_BASE + 0x038))
#define UARTICR   (*(volatile uint32_t *)(UART_BASE + 0x044))

// UARTFR bits — DDI 0183G Section 3.3.3
#define FR_TXFF  (1 << 5)
#define FR_RXFE  (1 << 4)
#define FR_BUSY  (1 << 3)

// UARTLCR_H bits — DDI 0183G Section 3.3.7
#define LCR_WLEN_8BIT (0b11 << 5)
#define LCR_FEN       (1 << 4)

// UARTCR bits — DDI 0183G Section 3.3.8
#define CR_UARTEN (1 << 0)
#define CR_TXE    (1 << 8)
#define CR_RXE    (1 << 9)

// UARTIMSC bits — DDI 0183G Section 3.3.10
#define IMSC_RXIM (1 << 4)
#define IMSC_RTIM (1 << 6)

// ---------------------------------------------------------------------------
// RX ring buffer — single-producer (IRQ handler) / single-consumer (reader)
// ---------------------------------------------------------------------------

#define RX_BUF_SIZE 64

static char rx_buf[RX_BUF_SIZE];
static volatile uint32_t rx_head;
static volatile uint32_t rx_tail;
static wait_queue_t rx_wq = WAIT_QUEUE_INIT;

// ---------------------------------------------------------------------------
// HAL implementation
// ---------------------------------------------------------------------------

void hal_uart_init(void)
{
    // DDI 0183G Section 3.3.8: disable UART before reconfiguration.
    UARTCR = 0;

    while (UARTFR & FR_BUSY)
        ;

    // Baud rate 115200. QEMU virt PL011 uses a 24 MHz reference clock.
    // Divisor = 24000000 / (16 * 115200) = 13.0208...
    UARTIBRD = 13;
    UARTFBRD = 1;

    // 8 data bits, no parity, 1 stop bit, FIFOs enabled.
    UARTLCR_H = LCR_WLEN_8BIT | LCR_FEN;

    // Clear all pending interrupts.
    UARTICR = 0x7FF;

    // Enable UART, TX, and RX.
    UARTCR = CR_UARTEN | CR_TXE | CR_RXE;

    // Enable GIC interrupt for UART RX.
    hal_irq_enable(HAL_IRQ_UART);

    // Unmask RX and Receive Timeout interrupts — DDI 0183G §3.3.10.
    UARTIMSC = IMSC_RXIM | IMSC_RTIM;
}

void hal_uart_putc(char c)
{
    while (UARTFR & FR_TXFF)
        ;
    UARTDR = (uint32_t)c & 0xFF;
}

char hal_uart_getc(void)
{
    __asm__ volatile("msr daifset, #2");

    while (rx_head == rx_tail) {
        process_sleep(&rx_wq);
        __asm__ volatile("msr daifset, #2");
    }

    char c = rx_buf[rx_tail];
    rx_tail = (rx_tail + 1) % RX_BUF_SIZE;

    __asm__ volatile("msr daifclr, #2");
    return c;
}

void hal_uart_puts(const char *str)
{
    for (; *str; str++) {
        if (*str == '\n')
            hal_uart_putc('\r');
        hal_uart_putc(*str);
    }
}

void hal_uart_irq_handler(void)
{
    // Drain all available characters from the RX FIFO.
    while (!(UARTFR & FR_RXFE)) {
        char c = (char)(UARTDR & 0xFF);
        uint32_t next_head = (rx_head + 1) % RX_BUF_SIZE;
        if (next_head != rx_tail) {
            rx_buf[rx_head] = c;
            rx_head = next_head;
        }
        // Feed the character to the TTY layer so stdin consumers receive
        // serial input regardless of whether a keyboard is also present.
        tty_receive_char(c);
    }

    // Clear both RX and timeout interrupts AFTER draining the FIFO.
    UARTICR = IMSC_RXIM | IMSC_RTIM;

    // Wake one blocked reader (if any).
    process_wakeup(&rx_wq);
}
