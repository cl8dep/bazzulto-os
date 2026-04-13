// platform/qemu_virt/gicv2.rs — ARM GICv2 interrupt controller driver.
//
// The GICv2 consists of two logical components:
//   - Distributor (GICD): global, manages interrupt routing and priority for all CPUs.
//   - CPU Interface (GICC): per-CPU, used to acknowledge and end interrupts.
//
// QEMU virt MMIO addresses (verified from QEMU DTB — see CLAUDE.md):
//   GICD physical base: 0x08000000
//   GICC physical base: 0x08010000
//
// All accesses go through the HHDM: virt = phys + hhdm_offset.
//
// Reference: ARM GIC Architecture Specification IHI 0048B.
// Reference: QEMU hw/arm/virt.c for MMIO layout on QEMU virt machine.

use core::ptr::{read_volatile, write_volatile};

// ---------------------------------------------------------------------------
// Interrupt IDs (INTID)
//
// GICv2 INTID ranges:
//   0–15   SGI (Software Generated Interrupts)
//   16–31  PPI (Private Peripheral Interrupts, per-CPU)
//   32–    SPI (Shared Peripheral Interrupts)
//
// Reference: IHI 0048B §2.2.1.
// ---------------------------------------------------------------------------

/// EL1 physical timer interrupt — PPI 14, INTID 30.
///
/// The ARM Generic Timer PPI for the EL1 physical timer is always INTID 30
/// on GICv2, regardless of SoC.  This is defined by the ARM architecture,
/// not QEMU.
/// Reference: ARM ARM DDI 0487 D11.2 "Generic Timer", Table D11-1 (PPI assignments).
pub const INTID_TIMER: u32 = 30;

/// PL011 UART0 interrupt — SPI 1, INTID 33.
///
/// QEMU virt maps UART0 to SPI 1 (INTID 32 + 1 = 33).
/// Reference: QEMU hw/arm/virt.c, virt_irqmap[], entry for UART.
pub const INTID_UART: u32 = 33;

/// Spurious interrupt identifier returned by GICC_IAR when no real interrupt
/// is pending.  Software must NOT send EOI for a spurious interrupt.
/// Reference: IHI 0048B §3.3 "Spurious interrupt".
pub const INTID_SPURIOUS: u32 = 1023;

/// First virtio-mmio SPI INTID.  Slot N maps to INTID_VIRTIO_BASE + N.
///
/// QEMU virt maps 32 virtio-mmio slots to SPIs 16..47 (INTID 48..79).
/// Formula: slot N → SPI (16 + N) → INTID (32 + 16 + N) = INTID (48 + N).
/// Reference: QEMU hw/arm/virt.c, virt_irqmap[], virtio-mmio entries.
pub const INTID_VIRTIO_BASE: u32 = 48;

// ---------------------------------------------------------------------------
// GICD (Distributor) register offsets
// Reference: IHI 0048B §4.3.
// ---------------------------------------------------------------------------

/// GICD_CTLR — Distributor Control Register.  Bit [0] = Enable.
/// IHI 0048B §4.3.1.
const GICD_CTLR: usize = 0x000;

/// GICD_ISENABLER — Interrupt Set-Enable Registers (one bit per INTID).
/// Register n covers INTIDs 32n … 32n+31.
/// IHI 0048B §4.3.5.
const GICD_ISENABLER_BASE: usize = 0x100;

/// GICD_IPRIORITYR — Interrupt Priority Registers (one byte per INTID).
/// Four INTIDs per 32-bit register.  Lower value = higher priority.
/// IHI 0048B §4.3.11.
const GICD_IPRIORITYR_BASE: usize = 0x400;

/// GICD_ITARGETSR — Interrupt Processor Targets Registers (one byte per INTID).
/// Each byte is a CPU target bitmask (bit N = CPU N).
/// Note: PPIs (INTID 0–31) have read-only ITARGETSR; only write for SPIs (32+).
/// IHI 0048B §4.3.12.
const GICD_ITARGETSR_BASE: usize = 0x800;

// ---------------------------------------------------------------------------
// GICC (CPU Interface) register offsets
// Reference: IHI 0048B §4.4.
// ---------------------------------------------------------------------------

/// GICC_CTLR — CPU Interface Control Register.  Bit [0] = Enable.
/// IHI 0048B §4.4.1.
const GICC_CTLR: usize = 0x000;

/// GICC_PMR — Interrupt Priority Mask Register.
/// The CPU interface forwards only interrupts with priority < PMR value.
/// 0xFF = allow all priorities.
/// IHI 0048B §4.4.2.
const GICC_PMR: usize = 0x004;

/// GICC_IAR — Interrupt Acknowledge Register.
/// Reading this register acknowledges the highest-priority pending interrupt
/// and returns its INTID in bits [9:0].
/// IHI 0048B §4.4.4.
const GICC_IAR: usize = 0x00C;

/// GICC_EOIR — End of Interrupt Register.
/// Write the INTID here to signal end-of-interrupt.
/// Must NOT be written for spurious interrupts (INTID 1023).
/// IHI 0048B §4.4.5.
const GICC_EOIR: usize = 0x010;

// ---------------------------------------------------------------------------
// Priority value constants
// ---------------------------------------------------------------------------

/// Highest possible GIC priority (value 0).  Lower value = higher priority.
const PRIORITY_HIGHEST: u32 = 0x00;

/// Default priority for all non-timer interrupts.
const PRIORITY_DEFAULT: u32 = 0xA0;

/// CPU interface priority mask: allow all interrupts (priority < 0xFF).
/// Reference: IHI 0048B §4.4.2.
const PRIORITY_MASK_ALL: u32 = 0xFF;

// ---------------------------------------------------------------------------
// GicV2 — driver state
// ---------------------------------------------------------------------------

pub struct GicV2 {
    /// Virtual base address of the GIC distributor (phys 0x08000000 + hhdm).
    gicd_base: usize,
    /// Virtual base address of the GIC CPU interface (phys 0x08010000 + hhdm).
    gicc_base: usize,
}

impl GicV2 {
    /// Construct a GICv2 driver instance.
    ///
    /// `gicd_phys` and `gicc_phys` are the physical base addresses of the
    /// distributor and CPU interface, respectively.  `hhdm_offset` is the
    /// Higher-Half Direct Map offset used to access physical memory.
    ///
    /// For QEMU virt: `gicd_phys = 0x08000000`, `gicc_phys = 0x08010000`.
    pub const fn new(gicd_phys: u64, gicc_phys: u64, hhdm_offset: u64) -> Self {
        Self {
            gicd_base: (gicd_phys + hhdm_offset) as usize,
            gicc_base: (gicc_phys + hhdm_offset) as usize,
        }
    }

    /// Initialise the GIC distributor and CPU interface.
    ///
    /// Initialization order per IHI 0048B §4.4.2:
    ///   1. Disable the distributor (GICD_CTLR = 0).
    ///   2. Set default priorities for all SPIs.
    ///   3. Route all SPIs to CPU 0.
    ///   4. Enable the distributor (GICD_CTLR = 1).
    ///   5. Set the priority mask to allow all interrupts.
    ///   6. Enable the CPU interface (GICC_CTLR = 1).
    ///
    /// Specific interrupt enables are done by `enable_interrupt()` afterwards.
    ///
    /// # Safety
    /// Must be called from EL1, exactly once, before any interrupts are unmasked.
    pub unsafe fn init(&self) {
        // Step 1: disable the distributor during configuration.
        // IHI 0048B §4.3.1.
        self.gicd_write(GICD_CTLR, 0);

        // Step 2: set all SPIs to a default medium priority.
        // INTIDs 0–31 are SGIs/PPIs; their priority registers are banked
        // per-CPU and will be configured when specific PPIs are enabled.
        // We configure SPIs (INTID 32+) here: 4 priorities per 32-bit word.
        // IHI 0048B §4.3.11.
        let spi_count = self.read_spi_count();
        for i in 0..spi_count {
            let intid = 32 + i;
            self.set_priority(intid, PRIORITY_DEFAULT);
        }

        // Step 3: route all SPIs to CPU 0.
        // PPIs (INTID 16–31) have read-only ITARGETSR; skip them.
        // IHI 0048B §4.3.12.
        for i in 0..spi_count {
            let intid = 32 + i;
            self.set_target(intid, 0x01); // CPU 0 target mask
        }

        // Step 4: enable the distributor.
        self.gicd_write(GICD_CTLR, 1);

        // Step 5: set CPU interface priority mask to allow all priorities.
        // IHI 0048B §4.4.2.
        self.gicc_write(GICC_PMR, PRIORITY_MASK_ALL);

        // Step 6: enable the CPU interface.
        // IHI 0048B §4.4.1.
        self.gicc_write(GICC_CTLR, 1);
    }

    /// Enable a single interrupt and configure its priority.
    ///
    /// `intid`: the interrupt ID to enable (0–1022; 1023 is spurious).
    /// `priority`: 0 = highest, 0xFF = lowest.
    ///
    /// For PPIs (INTID 16–31), ITARGETSR is read-only; target routing is
    /// not written.  For SPIs (INTID 32+) the interrupt is routed to CPU 0.
    ///
    /// # Panics
    /// Panics (in debug builds) if `intid >= 1023`.
    pub unsafe fn enable_interrupt(&self, intid: u32, priority: u32) {
        debug_assert!(intid < INTID_SPURIOUS, "invalid INTID");

        // Set priority.
        self.set_priority(intid, priority);

        // Route SPI to CPU 0 (skip PPIs — their ITARGETSR is read-only).
        if intid >= 32 {
            self.set_target(intid, 0x01);
        }

        // Set-enable: write 1 to the bit corresponding to `intid`.
        // GICD_ISENABLER register n covers INTIDs 32n … 32n+31.
        // IHI 0048B §4.3.5.
        let reg_index = (intid / 32) as usize;
        let bit_mask  = 1u32 << (intid % 32);
        self.gicd_write(GICD_ISENABLER_BASE + reg_index * 4, bit_mask);
    }

    /// Initialise the GIC CPU interface for the calling core.
    ///
    /// Called by each AP during SMP bringup.  The distributor (GICD) is
    /// already configured by the BSP's `init()` call; this function only
    /// re-runs the per-CPU steps (steps 5 and 6 of the full init sequence):
    ///
    ///   - Set the priority mask to allow all priorities (GICC_PMR = 0xFF).
    ///   - Enable the CPU interface (GICC_CTLR = 1).
    ///
    /// The EL1 physical timer PPI (INTID 30) is a banked PPI; it is enabled
    /// in the per-CPU GICD_ISENABLER[0] and GICD_IPRIORITYR registers here
    /// so that the timer interrupt is delivered to this core.
    ///
    /// # Safety
    /// Must be called from EL1 on the AP being initialised, after the BSP
    /// has completed `init()`.
    pub unsafe fn cpu_interface_init(&self) {
        // Set the priority mask to allow all interrupts.
        // IHI 0048B §4.4.2.
        self.gicc_write(GICC_PMR, PRIORITY_MASK_ALL);

        // Enable the CPU interface on this core.
        // IHI 0048B §4.4.1.
        self.gicc_write(GICC_CTLR, 1);
    }

    /// Acknowledge the highest-priority pending interrupt.
    ///
    /// Returns the INTID.  If no interrupt is pending, returns `INTID_SPURIOUS`.
    /// The caller MUST call `end_of_interrupt()` with the same INTID afterwards
    /// (unless the INTID is `INTID_SPURIOUS`).
    ///
    /// # Safety
    /// Must be called from an IRQ handler context at EL1.
    pub unsafe fn acknowledge(&self) -> u32 {
        // GICC_IAR bits [9:0] contain the INTID.  Bits [12:10] contain the
        // source CPU ID (for SGIs) — we discard those here.
        // IHI 0048B §4.4.4.
        self.gicc_read(GICC_IAR) & 0x3FF
    }

    /// Signal end-of-interrupt for `intid`.
    ///
    /// Must be called after processing a non-spurious interrupt.
    /// Calling EOI for `INTID_SPURIOUS` (1023) is incorrect per spec.
    ///
    /// # Safety
    /// Must be called from the same IRQ handler that called `acknowledge()`.
    pub unsafe fn end_of_interrupt(&self, intid: u32) {
        debug_assert_ne!(intid, INTID_SPURIOUS, "must not EOI spurious interrupt");
        // IHI 0048B §4.4.5.
        self.gicc_write(GICC_EOIR, intid);
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Read the number of SPIs from GICD_TYPER.ITLinesNumber [4:0].
    ///
    /// The total number of supported INTIDs is 32 × (ITLinesNumber + 1),
    /// minus the 32 SGI/PPI slots → SPI count = 32 × ITLinesNumber.
    /// IHI 0048B §4.3.2 GICD_TYPER.
    fn read_spi_count(&self) -> u32 {
        // GICD_TYPER offset 0x004.
        let typer = unsafe { self.gicd_read(0x004) };
        let it_lines = typer & 0x1F; // bits [4:0] = ITLinesNumber
        it_lines * 32
    }

    /// Write a priority value for `intid` into GICD_IPRIORITYR.
    ///
    /// Four INTIDs share one 32-bit register; each occupies 8 bits.
    /// IHI 0048B §4.3.11.
    unsafe fn set_priority(&self, intid: u32, priority: u32) {
        let reg_offset = GICD_IPRIORITYR_BASE + (intid as usize / 4) * 4;
        let byte_shift = (intid % 4) * 8;
        let mut val = self.gicd_read(reg_offset);
        val &= !(0xFF << byte_shift);
        val |= (priority & 0xFF) << byte_shift;
        self.gicd_write(reg_offset, val);
    }

    /// Write a CPU target mask for `intid` into GICD_ITARGETSR.
    ///
    /// Four INTIDs share one 32-bit register; each occupies 8 bits.
    /// `mask` is a bitmask where bit N = CPU N (e.g. 0x01 = CPU 0).
    /// IHI 0048B §4.3.12.
    unsafe fn set_target(&self, intid: u32, mask: u32) {
        let reg_offset = GICD_ITARGETSR_BASE + (intid as usize / 4) * 4;
        let byte_shift = (intid % 4) * 8;
        let mut val = self.gicd_read(reg_offset);
        val &= !(0xFF << byte_shift);
        val |= (mask & 0xFF) << byte_shift;
        self.gicd_write(reg_offset, val);
    }

    #[inline]
    unsafe fn gicd_read(&self, offset: usize) -> u32 {
        read_volatile((self.gicd_base + offset) as *const u32)
    }

    #[inline]
    unsafe fn gicd_write(&self, offset: usize, value: u32) {
        write_volatile((self.gicd_base + offset) as *mut u32, value);
    }

    #[inline]
    unsafe fn gicc_read(&self, offset: usize) -> u32 {
        read_volatile((self.gicc_base + offset) as *const u32)
    }

    #[inline]
    unsafe fn gicc_write(&self, offset: usize, value: u32) {
        write_volatile((self.gicc_base + offset) as *mut u32, value);
    }
}

// ---------------------------------------------------------------------------
// Convenience priority constants for callers
// ---------------------------------------------------------------------------

/// Priority value to use for the timer interrupt (highest).
pub const PRIORITY_TIMER: u32 = PRIORITY_HIGHEST;
/// Priority value to use for UART and other peripheral interrupts.
pub const PRIORITY_PERIPHERAL: u32 = PRIORITY_DEFAULT;
