// platform/qemu_virt/gicv3.rs — ARM GICv3 interrupt controller driver.
//
// GICv3 splits the interrupt controller into:
//   - Distributor (GICD): global, configures and enables SPIs.
//   - Redistributor (GICR): per-CPU, configures PPIs and SGIs.
//   - CPU Interface: accessed via system registers (ICC_*_EL1), not MMIO.
//
// Reference: ARM GICv3 and GICv4 Architecture Specification IHI 0069.

use core::ptr::{read_volatile, write_volatile};
use core::arch::asm;

// ---------------------------------------------------------------------------
// GICD register offsets
// Reference: IHI 0069 §8.9 (GICD registers).
// ---------------------------------------------------------------------------

/// GICD_CTLR — Distributor Control Register.
/// Bit [0] = EnableGrp0, bit [1] = EnableGrp1NS.
/// IHI 0069 §8.9.4.
const GICD_CTLR: usize = 0x000;

/// GICD_ISENABLER — Set-Enable Registers (one bit per INTID).
/// Register n covers INTIDs 32n … 32n+31.
/// IHI 0069 §8.9.7.
const GICD_ISENABLER_BASE: usize = 0x100;

/// GICD_IPRIORITYR — Priority Registers (one byte per INTID).
/// IHI 0069 §8.9.11.
const GICD_IPRIORITYR_BASE: usize = 0x400;

/// GICD_ICFGR — Interrupt Configuration Registers (2 bits per INTID).
/// Bit 1 = 1 → edge-triggered; bit 1 = 0 → level-sensitive.
/// IHI 0069 §8.9.6.
const GICD_ICFGR_BASE: usize = 0xC00;

// ---------------------------------------------------------------------------
// GICR register offsets
// The redistributor has two 64 KiB frames per CPU:
//   Frame 0 (RD frame):  GICR_base + 0x00000  — wake / identity registers
//   Frame 1 (SGI frame): GICR_base + 0x10000  — SGI/PPI enable/priority
// Reference: IHI 0069 §9.
// ---------------------------------------------------------------------------

/// GICR_WAKER offset within the RD frame.
/// Bit [1] = ProcessorSleep (write 0 to wake).
/// Bit [2] = ChildrenAsleep (poll until 0 after clearing ProcessorSleep).
/// IHI 0069 §9.3.12.
const GICR_WAKER: usize = 0x014;

/// Byte offset of the SGI/PPI frame from the start of one CPU's GICR block.
const GICR_SGI_FRAME_OFFSET: usize = 0x10000;

/// GICR_ISENABLER0 offset within the SGI/PPI frame.
/// Controls enable state of SGIs (bits 0–15) and PPIs (bits 16–31).
/// IHI 0069 §9.3.5.
const GICR_ISENABLER0: usize = 0x100;

/// GICR_IPRIORITYR base offset within the SGI/PPI frame.
/// One byte per INTID, covering INTIDs 0–31.
/// IHI 0069 §9.3.7.
const GICR_IPRIORITYR_BASE: usize = 0x400;

// ---------------------------------------------------------------------------
// GICD_CTLR value
// ---------------------------------------------------------------------------

/// Enable Group 0 and Group 1 NS interrupts.
/// IHI 0069 §8.9.4.
const GICD_CTLR_ENABLE_GRP0_GRP1NS: u32 = 0x03;

// ---------------------------------------------------------------------------
// Priority constants
// ---------------------------------------------------------------------------

/// Allow all interrupt priorities (CPU interface priority mask).
/// IHI 0069 §9.5.1 ICC_PMR_EL1.
const PRIORITY_MASK_ALL: u64 = 0xFF;

/// Highest priority (value 0).
const PRIORITY_HIGHEST: u8 = 0x00;

/// Default priority for peripheral interrupts.
const PRIORITY_DEFAULT: u8 = 0xA0;

/// Priority for the timer interrupt (highest, so preemption works).
pub const PRIORITY_TIMER: u8 = PRIORITY_HIGHEST;

/// Priority for other peripheral interrupts.
pub const PRIORITY_PERIPHERAL: u8 = PRIORITY_DEFAULT;

// ---------------------------------------------------------------------------
// GicV3 — driver state
// ---------------------------------------------------------------------------

pub struct GicV3 {
    /// Virtual base address of the GIC Distributor.
    gicd_virt_base: usize,
    /// Virtual base address of the GIC Redistributor (CPU 0 block).
    gicr_virt_base: usize,
}

impl GicV3 {
    /// Construct a GICv3 driver from physical base addresses and HHDM offset.
    ///
    /// # Safety
    /// `gicd_phys` and `gicr_phys` must be the correct physical addresses for
    /// the GICv3 on this platform.  `hhdm_offset` must be the HHDM direct-map
    /// offset so that `phys + hhdm_offset` is a valid virtual address.
    pub unsafe fn new(gicd_phys: u64, gicr_phys: u64, hhdm_offset: u64) -> Self {
        Self {
            gicd_virt_base: (gicd_phys + hhdm_offset) as usize,
            gicr_virt_base: (gicr_phys + hhdm_offset) as usize,
        }
    }

    /// Perform the full GICv3 initialisation sequence.
    ///
    /// Sequence per IHI 0069 §4.1.1:
    ///   1. Enable ICC_SRE_EL1 (system register interface).
    ///   2. Wake the redistributor (clear GICR_WAKER.ProcessorSleep).
    ///   3. Disable the distributor; configure SPI priorities; re-enable.
    ///   4. Set ICC_PMR_EL1 to allow all priorities.
    ///   5. Set ICC_BPR1_EL1 = 0 (all priority bits used as group priority).
    ///   6. Enable Group 1 NS interrupts via ICC_IGRPEN1_EL1.
    ///
    /// # Safety
    /// Must be called from EL1, exactly once, before any interrupts are unmasked.
    pub unsafe fn init(&self) {
        // Step 1: Enable the system register interface.
        // ICC_SRE_EL1 bit [0] = SRE.  Must be set before any other ICC_* access.
        // IHI 0069 §8.5.18 ICC_SRE_EL1.
        let mut sre: u64;
        asm!("mrs {}, ICC_SRE_EL1", out(reg) sre);
        sre |= 1;
        asm!("msr ICC_SRE_EL1, {}", in(reg) sre);
        asm!("isb");

        // Step 2: Wake the redistributor for CPU 0.
        // Clear ProcessorSleep (bit 1) and poll until ChildrenAsleep (bit 2) = 0.
        // IHI 0069 §9.3.12.
        let waker_addr = (self.gicr_virt_base + GICR_WAKER) as *mut u32;
        let mut waker = read_volatile(waker_addr);
        waker &= !2; // clear ProcessorSleep
        write_volatile(waker_addr, waker);
        // Poll until ChildrenAsleep = 0.
        loop {
            let w = read_volatile(waker_addr);
            if w & 4 == 0 {
                break;
            }
        }

        // Step 3: Configure and enable the distributor.
        // Disable first so we can safely write configuration registers.
        self.gicd_write(GICD_CTLR, 0);

        let spi_count = self.read_spi_count();
        for i in 0..spi_count {
            let intid = 32 + i;
            self.set_spi_priority(intid, PRIORITY_DEFAULT);
        }

        self.gicd_write(GICD_CTLR, GICD_CTLR_ENABLE_GRP0_GRP1NS);

        // Step 4: Set priority mask to allow all interrupts.
        // ICC_PMR_EL1.  IHI 0069 §8.5.17.
        asm!("msr ICC_PMR_EL1, {}", in(reg) PRIORITY_MASK_ALL);

        // Step 5: Set binary point register so all bits determine group priority.
        // ICC_BPR1_EL1 = 0.  IHI 0069 §8.5.2.
        asm!("msr ICC_BPR1_EL1, xzr");

        // Step 6: Enable Group 1 NS interrupts.
        // ICC_IGRPEN1_EL1 bit [0] = EnableGrp1.  IHI 0069 §8.5.7.
        asm!("msr ICC_IGRPEN1_EL1, {}", in(reg) 1u64);

        asm!("isb");
    }

    /// Enable a single interrupt at the given `intid` with the given `priority`.
    ///
    /// For PPIs (INTID 16–31): configures via GICR SGI/PPI frame.
    /// For SPIs (INTID 32+):   configures via GICD.
    ///
    /// # Safety
    /// Must be called after `init`.
    pub unsafe fn enable_interrupt(&self, intid: u32, priority: u8) {
        if intid < 32 {
            // PPI/SGI: configure in the GICR SGI frame.
            self.set_ppi_priority(intid, priority);
            let sgi_base = self.gicr_virt_base + GICR_SGI_FRAME_OFFSET;
            let reg_index = (intid / 32) as usize;
            let bit_mask  = 1u32 << (intid % 32);
            write_volatile(
                (sgi_base + GICR_ISENABLER0 + reg_index * 4) as *mut u32,
                bit_mask,
            );
        } else {
            // SPI: configure in the GICD.
            self.set_spi_priority(intid, priority);
            let reg_index = (intid / 32) as usize;
            let bit_mask  = 1u32 << (intid % 32);
            self.gicd_write(GICD_ISENABLER_BASE + reg_index * 4, bit_mask);
        }
    }

    /// Set the priority of a single interrupt by `intid`.
    ///
    /// # Safety
    /// Must be called after `init`.
    pub unsafe fn set_priority(&self, intid: u32, priority: u8) {
        if intid < 32 {
            self.set_ppi_priority(intid, priority);
        } else {
            self.set_spi_priority(intid, priority);
        }
    }

    /// Read ICC_IAR1_EL1 to acknowledge the highest-priority Group 1 interrupt.
    ///
    /// Returns the INTID.  A value of 1023 means no interrupt was pending
    /// (spurious).
    ///
    /// # Safety
    /// Must be called from an IRQ handler context at EL1.
    pub unsafe fn acknowledge(&self) -> u32 {
        let intid: u64;
        // IHI 0069 §8.5.6 ICC_IAR1_EL1.
        asm!("mrs {}, ICC_IAR1_EL1", out(reg) intid);
        (intid & 0xFFFFFF) as u32 // bits [23:0] = INTID
    }

    /// Write ICC_EOIR1_EL1 to signal end-of-interrupt for `intid`.
    ///
    /// # Safety
    /// Must be called after `acknowledge` returns a non-spurious INTID.
    pub unsafe fn end_of_interrupt(&self, intid: u32) {
        // IHI 0069 §8.5.4 ICC_EOIR1_EL1.
        asm!("msr ICC_EOIR1_EL1, {}", in(reg) intid as u64);
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Read GICD_TYPER to determine the number of SPI lines.
    fn read_spi_count(&self) -> u32 {
        // GICD_TYPER at offset 0x004.  Bits [4:0] = ITLinesNumber.
        // SPI count = ITLinesNumber * 32.
        // IHI 0069 §8.9.2.
        let typer = unsafe { self.gicd_read(0x004) };
        let it_lines = typer & 0x1F;
        it_lines * 32
    }

    /// Write priority for an SPI (INTID >= 32) into GICD_IPRIORITYR.
    unsafe fn set_spi_priority(&self, intid: u32, priority: u8) {
        let reg_offset = GICD_IPRIORITYR_BASE + (intid as usize / 4) * 4;
        let byte_shift = (intid % 4) * 8;
        let mut val = self.gicd_read(reg_offset);
        val &= !(0xFF << byte_shift);
        val |= (priority as u32) << byte_shift;
        self.gicd_write(reg_offset, val);
    }

    /// Write priority for a PPI/SGI (INTID < 32) into GICR_IPRIORITYR.
    unsafe fn set_ppi_priority(&self, intid: u32, priority: u8) {
        let sgi_base   = self.gicr_virt_base + GICR_SGI_FRAME_OFFSET;
        let reg_offset = GICR_IPRIORITYR_BASE + (intid as usize / 4) * 4;
        let byte_shift = (intid % 4) * 8;
        let ptr        = (sgi_base + reg_offset) as *mut u32;
        let mut val    = read_volatile(ptr);
        val &= !(0xFF << byte_shift);
        val |= (priority as u32) << byte_shift;
        write_volatile(ptr, val);
    }

    #[inline]
    unsafe fn gicd_read(&self, offset: usize) -> u32 {
        read_volatile((self.gicd_virt_base + offset) as *const u32)
    }

    #[inline]
    unsafe fn gicd_write(&self, offset: usize, value: u32) {
        write_volatile((self.gicd_virt_base + offset) as *mut u32, value);
    }
}
