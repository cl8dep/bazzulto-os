// GICv2 interrupt controller backend for QEMU virt — IHI 0048B
//
// QEMU virt machine maps:
//   Distributor  at physical 0x08000000
//   CPU Interface at physical 0x08010000
// Source: QEMU hw/arm/virt.c

#include "../../../include/bazzulto/hal/hal_irq.h"
#include "../../../include/bazzulto/kernel.h"

// ---------------------------------------------------------------------------
// GICv2 register interface — IHI 0048B (ARM GIC Architecture Specification)
// ---------------------------------------------------------------------------

#define GICD_PHYS_BASE  0x08000000ULL
#define GICC_PHYS_BASE  0x08010000ULL

#define GICD_BASE  (hhdm_offset + GICD_PHYS_BASE)
#define GICC_BASE  (hhdm_offset + GICC_PHYS_BASE)

// Distributor registers — IHI 0048B Section 4.3
#define GICD_CTLR          (*(volatile uint32_t *)(GICD_BASE + 0x000))
#define GICD_ISENABLER(n)  (*(volatile uint32_t *)(GICD_BASE + 0x100 + 4 * (n)))
#define GICD_IPRIORITYR    ((volatile uint32_t *)(GICD_BASE + 0x400))
#define GICD_ITARGETSR     ((volatile uint32_t *)(GICD_BASE + 0x800))

// CPU Interface registers — IHI 0048B Section 4.4
#define GICC_CTLR  (*(volatile uint32_t *)(GICC_BASE + 0x000))
#define GICC_PMR   (*(volatile uint32_t *)(GICC_BASE + 0x004))
#define GICC_IAR   (*(volatile uint32_t *)(GICC_BASE + 0x00C))
#define GICC_EOIR  (*(volatile uint32_t *)(GICC_BASE + 0x010))

// ---------------------------------------------------------------------------
// Well-known interrupt IDs for QEMU virt
// ---------------------------------------------------------------------------

const uint32_t HAL_IRQ_TIMER    = 30;   // EL1 Physical Timer PPI (ARM arch fixed)
const uint32_t HAL_IRQ_UART     = 33;   // PL011 UART0 SPI (QEMU virt: SPI 1)
const uint32_t HAL_IRQ_SPURIOUS = 1023; // Spurious — do NOT write EOIR

// Virtio-mmio IRQ base — QEMU virt wires slot N to SPI (16+N) = INTID (48+N).
// Source: QEMU hw/arm/virt.c
#define IRQ_VIRTIO_MMIO_BASE  48

// ---------------------------------------------------------------------------
// HAL implementation
// ---------------------------------------------------------------------------

void hal_irq_init(void)
{
    // GIC initialization order per IHI 0048B Section 4.4.2:
    //   1. Disable distributor
    //   2. Configure interrupt properties
    //   3. Enable distributor
    //   4. Configure CPU interface

    GICD_CTLR = 0;

    // Configure INTID 30 (EL1 Physical Timer PPI).
    // PPIs (INTIDs 16-31) have fixed routing — ITARGETSR is read-only.
    // Set priority to 0 (highest).
    GICD_IPRIORITYR[30 / 4] = 0x00000000;

    // Enable INTID 30 in ISENABLER0 (INTIDs 0-31).
    GICD_ISENABLER(0) = (1U << HAL_IRQ_TIMER);

    // Enable distributor and CPU interface.
    GICD_CTLR = 1;
    GICC_PMR  = 0xFF;
    GICC_CTLR = 1;
}

void hal_irq_enable(uint32_t irq_id)
{
    // Set priority to 0 (highest) — IHI 0048B §4.3.11
    uint32_t priority_register = irq_id / 4;
    uint32_t priority_offset   = (irq_id % 4) * 8;
    GICD_IPRIORITYR[priority_register] &= ~(0xFFU << priority_offset);

    // Route to CPU 0 — IHI 0048B §4.3.12
    // For SPIs (INTID >= 32), ITARGETSR is read-write.
    uint32_t target_register = irq_id / 4;
    uint32_t target_offset   = (irq_id % 4) * 8;
    uint32_t targets = GICD_ITARGETSR[target_register];
    targets &= ~(0xFFU << target_offset);
    targets |= (0x01U << target_offset);
    GICD_ITARGETSR[target_register] = targets;

    // Enable the interrupt — IHI 0048B §4.3.5
    uint32_t enable_register = irq_id / 32;
    uint32_t enable_bit      = irq_id % 32;
    GICD_ISENABLER(enable_register) = (1U << enable_bit);
}

uint32_t hal_irq_acknowledge(void)
{
    return GICC_IAR & 0x3FFU;
}

void hal_irq_end(uint32_t irq_id)
{
    GICC_EOIR = irq_id;
}
