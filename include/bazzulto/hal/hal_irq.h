#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// HAL: Interrupt Controller
//
// Platform-independent interface for managing hardware interrupts.
// The platform backend (e.g. GICv2 for QEMU virt) implements these functions.
// ---------------------------------------------------------------------------

// Initialize the interrupt controller hardware.
// Must be called before any other hal_irq function.
void hal_irq_init(void);

// Enable a peripheral interrupt and route it to the boot CPU.
// irq_id is the platform-specific interrupt identifier (e.g. GIC INTID).
void hal_irq_enable(uint32_t irq_id);

// Acknowledge a pending interrupt. Returns the interrupt ID.
// Must be called exactly once at the start of every IRQ handler entry.
uint32_t hal_irq_acknowledge(void);

// Signal end-of-interrupt for a previously acknowledged IRQ.
// Must be called before returning from the IRQ handler.
// Do NOT call for spurious interrupts (HAL_IRQ_SPURIOUS).
void hal_irq_end(uint32_t irq_id);

// Well-known interrupt IDs. Defined as constants by the platform backend.
extern const uint32_t HAL_IRQ_TIMER;
extern const uint32_t HAL_IRQ_UART;
extern const uint32_t HAL_IRQ_SPURIOUS;
