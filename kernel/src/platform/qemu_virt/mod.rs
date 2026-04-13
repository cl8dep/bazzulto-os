// platform/qemu_virt/mod.rs — QEMU virt platform initialisation.
//
// This module owns the global GIC and Timer instances and provides the
// `irq_dispatch()` function called by the exception handlers.
//
// Physical MMIO addresses for QEMU virt (verified from QEMU-generated DTB
// — see CLAUDE.md "Verified machine facts"):
//   GIC distributor:   0x08000000
//   GIC CPU interface: 0x08010000
//   PL011 UART:        0x09000000
//
// These constants are the ONLY place in the Rust kernel where QEMU virt
// addresses appear; all other code receives them as parameters.

pub mod gicv2;
pub mod gicv3;
pub mod keyboard_virtio;
pub mod rtc;
pub mod timer;
pub mod virtio_blk;
pub mod virtio_mmio;

use core::cell::UnsafeCell;

use gicv2::{GicV2, INTID_SPURIOUS, INTID_TIMER};
use timer::Timer;

use crate::drivers::uart;
use crate::platform::dtb;
use crate::platform::platform_trait::{DetectedPlatform, platform_set};

// ---------------------------------------------------------------------------
// QEMU virt MMIO physical base addresses (fallback defaults)
// Reference: QEMU hw/arm/virt.c, virt_memmap[].
// ---------------------------------------------------------------------------

/// GIC Distributor physical base address on QEMU virt.
pub const GICD_PHYS_BASE: u64 = 0x08000000;

/// GIC CPU Interface physical base address on QEMU virt.
pub const GICC_PHYS_BASE: u64 = 0x08010000;

/// PL011 UART0 physical base address on QEMU virt.
pub const UART_PHYS_BASE: u64 = 0x09000000;

// ---------------------------------------------------------------------------
// Global platform state
// ---------------------------------------------------------------------------

/// Which GIC variant is active.
enum ActiveGic {
    Version2(GicV2),
    Version3(gicv3::GicV3),
}

/// Platform state holding the GIC and timer driver instances.
struct PlatformState {
    gic: ActiveGic,
    timer: Timer,
}

// SAFETY: single-core kernel; all accesses are from EL1 with IRQs managed
// by the caller.  SMP will require a spinlock here.
unsafe impl Sync for PlatformState {}

struct SyncCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static PLATFORM: SyncCell<Option<PlatformState>> = SyncCell(UnsafeCell::new(None));

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Initialise all platform hardware: GIC, then timer.
///
/// `hhdm_offset` is the HHDM direct-map offset for MMIO virtual address
/// calculation.
///
/// `dtb_addr` is the physical address of the Device Tree Blob provided by
/// Limine.  Pass 0 if no DTB is available; the QEMU virt defaults are used.
///
/// Must be called after the memory subsystem is up (HHDM mapped) so that
/// MMIO registers are accessible via the HHDM virtual addresses.
///
/// After this function returns:
///   - VBAR_EL1 is set to the Rust exception vector table.
///   - The GIC distributor and CPU interface are enabled.
///   - The EL1 physical timer interrupt (INTID 30) is enabled and armed.
///   - IRQs are unmasked (DAIF.I = 0).
///
/// # Safety
/// Must be called from a single-threaded EL1 context.
pub unsafe fn platform_init(hhdm_offset: u64, dtb_addr: u64) {
    // --- Step 1: detect hardware platform from DTB or fall back to defaults ---

    let dtb_info = if dtb_addr != 0 {
        let dtb_ptr = (dtb_addr + hhdm_offset) as *const u8;
        match dtb::parse_dtb(dtb_ptr) {
            Some(info) => {
                uart::puts("DTB: parsed successfully\r\n");
                info
            }
            None => {
                uart::puts("DTB: parse failed — using QEMU virt defaults\r\n");
                dtb::DtbInfo::qemu_virt_defaults()
            }
        }
    } else {
        uart::puts("DTB: not provided — using QEMU virt defaults\r\n");
        dtb::DtbInfo::qemu_virt_defaults()
    };

    // Store the detected platform description for use by other subsystems.
    platform_set(DetectedPlatform {
        uart_phys_base:     dtb_info.uart_phys_base,
        gicd_phys_base:     dtb_info.gicd_phys_base,
        gicc_phys_base:     dtb_info.gicc_phys_base,
        gicr_phys_base:     dtb_info.gicr_phys_base,
        gic_version:        dtb_info.gic_version,
        timer_intid:        dtb_info.timer_intid,
        total_memory_bytes: dtb_info.total_memory_bytes,
        cpu_count:          dtb_info.cpu_count,
    });

    // --- Step 2: enumerate the virtio-mmio bus ---
    virtio_mmio::enumerate(hhdm_offset);

    // --- Step 3: initialise the interrupt controller ---

    let active_gic = if dtb_info.gic_version == 3 && dtb_info.gicr_phys_base != 0 {
        uart::puts("GIC: GICv3 detected\r\n");
        let gic = gicv3::GicV3::new(
            dtb_info.gicd_phys_base,
            dtb_info.gicr_phys_base,
            hhdm_offset,
        );
        gic.init();
        // Enable the EL1 physical timer PPI (INTID 30).
        gic.enable_interrupt(INTID_TIMER, gicv3::PRIORITY_TIMER);
        ActiveGic::Version3(gic)
    } else {
        uart::puts("GIC: GICv2 detected\r\n");
        let gic = GicV2::new(
            dtb_info.gicd_phys_base,
            dtb_info.gicc_phys_base,
            hhdm_offset,
        );
        gic.init();
        gic.enable_interrupt(INTID_TIMER, gicv2::PRIORITY_TIMER);
        ActiveGic::Version2(gic)
    };

    uart::puts("GIC: initialised\r\n");

    // --- Step 4: initialise the timer ---
    let timer = Timer::init();
    uart::puts("Timer: armed\r\n");

    // --- Step 4b: initialise the PL031 RTC and record boot-time wall clock ---
    let pl031_rtc = rtc::Pl031Rtc::new(hhdm_offset);
    pl031_rtc.enable();
    let boot_rtc_seconds = pl031_rtc.read_seconds() as u64;
    rtc::set_boot_rtc_seconds(boot_rtc_seconds);
    // Also populate the vDSO data page so fast clock_gettime can read it from EL0.
    crate::vdso::vdso_set_boot_rtc_seconds(boot_rtc_seconds);
    uart::puts("RTC: PL031 enabled\r\n");

    *PLATFORM.0.get() = Some(PlatformState {
        gic: active_gic,
        timer,
    });

    // --- Step 5: install exception vectors and unmask IRQs ---
    crate::arch::arm64::exceptions::exceptions_init();

    uart::puts("Exceptions: VBAR_EL1 set, IRQs unmasked\r\n");
}

// ---------------------------------------------------------------------------
// IRQ dispatch — called from exception handlers in exceptions/mod.rs
// ---------------------------------------------------------------------------

/// Dispatch one pending interrupt from the GIC.
///
/// Called from `exception_handler_irq_el1` and `exception_handler_irq_el0`
/// in the exception handler module.
///
/// # Safety
/// Must be called from an EL1 IRQ handler context.
pub fn irq_dispatch() {
    let state = unsafe {
        (*PLATFORM.0.get())
            .as_ref()
            .expect("irq_dispatch called before platform_init")
    };

    let intid = unsafe {
        match &state.gic {
            ActiveGic::Version2(gic) => gic.acknowledge(),
            ActiveGic::Version3(gic) => gic.acknowledge(),
        }
    };

    match intid {
        INTID_SPURIOUS => {
            // Spurious interrupt — GIC indicated no pending interrupt.
            // Per IHI 0048B §3.3 / IHI 0069: do NOT send EOI for spurious interrupts.
        }
        INTID_TIMER => {
            unsafe { state.timer.handle_irq() };
            unsafe {
                match &state.gic {
                    ActiveGic::Version2(gic) => gic.end_of_interrupt(intid),
                    ActiveGic::Version3(gic) => gic.end_of_interrupt(intid),
                }
            };
        }
        other => {
            // Check if this is a virtio-blk or virtio-keyboard IRQ.
            let disk_irq = crate::platform::qemu_virt::virtio_blk::disk_get_irq_id();
            let keyboard_irq = crate::platform::qemu_virt::keyboard_virtio::keyboard_get_irq_id();

            if other == disk_irq && disk_irq != 0 {
                unsafe { crate::platform::qemu_virt::virtio_blk::disk_irq_handler() };
            } else if other == keyboard_irq && keyboard_irq != 0 {
                unsafe { crate::platform::qemu_virt::keyboard_virtio::keyboard_irq_handler() };
            } else {
                uart::puts("IRQ: unexpected INTID ");
                uart::put_hex(other as u64);
                uart::puts("\r\n");
            }
            unsafe {
                match &state.gic {
                    ActiveGic::Version2(gic) => gic.end_of_interrupt(other),
                    ActiveGic::Version3(gic) => gic.end_of_interrupt(other),
                }
            };
        }
    }
}

/// Initialise the GIC CPU interface for the calling AP core.
///
/// Called from `ap_entry()` on each secondary core.  The GIC distributor is
/// already globally configured by the BSP's `platform_init()` call.  This
/// function runs only the per-CPU portion (GICC_PMR and GICC_CTLR).
///
/// # Safety
/// Must be called after `platform_init()` has run on the BSP.
pub unsafe fn ap_gic_cpu_interface_init() {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("ap_gic_cpu_interface_init called before platform_init");
    match &state.gic {
        ActiveGic::Version2(gic) => gic.cpu_interface_init(),
        ActiveGic::Version3(_gic) => {
            // GICv3 redistributor initialisation for APs would go here.
            // Not yet implemented; GICv3 requires per-CPU GICR configuration.
            uart::puts("SMP: GICv3 AP CPU interface init not yet implemented\r\n");
        }
    }
}

/// Arm the EL1 physical timer on the calling AP core.
///
/// The Generic Timer's control and compare registers are banked per PE; each
/// core must arm its own timer.
/// Reference: ARM ARM DDI 0487 §D11.2 (CNTP_CTL_EL0 is banked).
///
/// # Safety
/// Must be called from EL1 after `platform_init()`.
pub unsafe fn ap_timer_arm_this_core() {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("ap_timer_arm_this_core called before platform_init");
    state.timer.arm_this_core();
}

/// Access the timer for delay operations.
///
/// # Safety
/// Must be called after `platform_init`.
pub unsafe fn timer() -> &'static Timer {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("timer() called before platform_init");
    &state.timer
}

// ---------------------------------------------------------------------------
// HAL delegation helpers — called from hal::irq, hal::timer, hal::disk, etc.
// ---------------------------------------------------------------------------

/// Return the list of MMIO regions that must be mapped in the kernel page table.
pub fn platform_mmio_regions() -> &'static [crate::hal::MmioRegion] {
    // GIC and UART regions are mapped by the memory subsystem already.
    // Return empty slice; MMIO is accessed through HHDM.
    &[]
}

/// Enable interrupt `intid` in the GIC distributor.
///
/// # Safety
/// Must be called after platform_init.
pub unsafe fn gic_enable_interrupt(intid: u32) {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("gic_enable_interrupt called before platform_init");
    match &state.gic {
        ActiveGic::Version2(gic) => gic.enable_interrupt(intid, gicv2::PRIORITY_PERIPHERAL),
        ActiveGic::Version3(gic) => gic.enable_interrupt(intid, gicv3::PRIORITY_PERIPHERAL),
    }
}

/// Acknowledge the highest-priority pending interrupt.
///
/// # Safety
/// Must be called from an IRQ handler.
pub unsafe fn gic_acknowledge() -> u32 {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("gic_acknowledge called before platform_init");
    match &state.gic {
        ActiveGic::Version2(gic) => gic.acknowledge(),
        ActiveGic::Version3(gic) => gic.acknowledge(),
    }
}

/// Signal end-of-interrupt.
///
/// # Safety
/// Must be called after handling the interrupt.
pub unsafe fn gic_end_of_interrupt(intid: u32) {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("gic_end_of_interrupt called before platform_init");
    match &state.gic {
        ActiveGic::Version2(gic) => gic.end_of_interrupt(intid),
        ActiveGic::Version3(gic) => gic.end_of_interrupt(intid),
    }
}

/// Handle the timer IRQ (rearm the timer).
///
/// # Safety
/// Must be called from an IRQ handler.
pub unsafe fn timer_handle_irq() {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("timer_handle_irq called before platform_init");
    state.timer.handle_irq();
}

/// Delay for `milliseconds` milliseconds using the timer.
///
/// # Safety
/// Must be called after platform_init.
pub unsafe fn timer_delay_ms(milliseconds: u64) {
    let state = (*PLATFORM.0.get())
        .as_ref()
        .expect("timer_delay_ms called before platform_init");
    state.timer.delay_ms(milliseconds);
}
