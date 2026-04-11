#pragma once

#include <stdint.h>
#include "kernel.h"

// ---------------------------------------------------------------------------
// GICv2 register interface — IHI 0048B (ARM GIC Architecture Specification)
//
// QEMU virt machine maps:
//   Distributor  at physical 0x08000000  (hw/arm/virt.c)
//   CPU Interface at physical 0x08010000
//
// All accesses go through the HHDM since the MMU is active.
// ---------------------------------------------------------------------------

#define GICD_BASE  (hhdm_offset + 0x08000000ULL)
#define GICC_BASE  (hhdm_offset + 0x08010000ULL)

// Distributor registers — IHI 0048B Section 4.3
#define GICD_CTLR        (*(volatile uint32_t *)(GICD_BASE + 0x000))
#define GICD_ISENABLER(n) (*(volatile uint32_t *)(GICD_BASE + 0x100 + 4 * (n)))
#define GICD_IPRIORITYR  ((volatile uint32_t *)(GICD_BASE + 0x400))
#define GICD_ITARGETSR   ((volatile uint32_t *)(GICD_BASE + 0x800))

// CPU Interface registers — IHI 0048B Section 4.4
#define GICC_CTLR  (*(volatile uint32_t *)(GICC_BASE + 0x000))
#define GICC_PMR   (*(volatile uint32_t *)(GICC_BASE + 0x004))
#define GICC_IAR   (*(volatile uint32_t *)(GICC_BASE + 0x00C))
#define GICC_EOIR  (*(volatile uint32_t *)(GICC_BASE + 0x010))

// Well-known interrupt IDs on QEMU virt
#define IRQ_TIMER_EL1_PHYS  30   // EL1 Physical Timer PPI (ARM arch fixed)
#define IRQ_UART0           33   // First PL011 UART SPI (QEMU virt: SPI 1 = INTID 33)
#define IRQ_SPURIOUS        1023 // Spurious interrupt — do NOT write EOIR

// virtio-mmio IRQ base — QEMU virt wires slot N to SPI (16+N) = INTID (48+N).
// Source: QEMU hw/arm/virt.c, VIRT_MMIO first IRQ = GIC_SPI(16), INTID = 32+16 = 48.
#define IRQ_VIRTIO_MMIO_BASE  48

// Enable an SPI (INTID >= 32) and route it to CPU 0.
// For PPIs (INTID 16-31), routing is fixed — do not call this.
static inline void gic_enable_spi(uint32_t intid) {
	// Set priority to 0 (highest) — IHI 0048B §4.3.11
	// Each register holds 4 priorities (1 byte each).
	uint32_t pri_reg = intid / 4;
	uint32_t pri_off = (intid % 4) * 8;
	GICD_IPRIORITYR[pri_reg] &= ~(0xFFU << pri_off);

	// Route to CPU 0 — IHI 0048B §4.3.12
	// For SPIs (INTID >= 32), ITARGETSR is read-write.
	uint32_t tgt_reg = intid / 4;
	uint32_t tgt_off = (intid % 4) * 8;
	uint32_t targets = GICD_ITARGETSR[tgt_reg];
	targets &= ~(0xFFU << tgt_off);
	targets |= (0x01U << tgt_off);
	GICD_ITARGETSR[tgt_reg] = targets;

	// Enable the interrupt — IHI 0048B §4.3.5
	// ISENABLER uses set-enable semantics (write-1-to-enable).
	uint32_t enable_reg = intid / 32;
	uint32_t enable_bit = intid % 32;
	GICD_ISENABLER(enable_reg) = (1U << enable_bit);
}
