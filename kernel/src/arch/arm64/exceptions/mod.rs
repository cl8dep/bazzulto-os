// arch/arm64/exceptions/mod.rs — AArch64 exception handling.
//
// Boot steps:
//   1. exceptions_init()   — write VBAR_EL1 and unmask interrupts (DAIF.I)
//   2. The GIC must be initialised before step 1 so that the first IRQ does
//      not arrive with no handler registered.
//
// Reference: ARM ARM DDI 0487 D1.10 "Exception handling".

use core::fmt;

use crate::drivers::uart;

// ---------------------------------------------------------------------------
// ExceptionFrame — matches the layout in vectors.S exactly.
//
// The compile_error! assertion below verifies the size at compile time.
// If a field is added or removed, the assembly offsets in vectors.S MUST be
// updated to match.
// ---------------------------------------------------------------------------

/// Saved CPU state at exception entry.
///
/// Layout: 36 × u64 = 288 bytes, matching the `save_exception_frame` macro.
///
/// # Safety
/// The assembly vectors.S pushes this struct onto the kernel stack and passes
/// a raw pointer to it as the first argument (x0) to every `extern "C"` handler.
/// Handlers MUST NOT move or copy the frame — only read or modify fields in place.
#[repr(C)]
pub struct ExceptionFrame {
    // General-purpose registers x0–x30.
    pub x: [u64; 31],
    /// Stack pointer at exception entry.
    ///   - EL1 exceptions: SP_EL1 before the frame was pushed.
    ///   - EL0 exceptions: SP_EL0 (user stack pointer).
    pub sp: u64,
    /// Exception Link Register — program counter at exception entry (ELR_EL1).
    pub elr: u64,
    /// Saved Processor State Register (SPSR_EL1).
    pub spsr: u64,
    /// Exception Syndrome Register (ESR_EL1).
    pub esr: u64,
    /// Fault Address Register (FAR_EL1).
    pub far: u64,
}

// Compile-time check: frame must be exactly 288 bytes.
// If this fails, the assembly offsets in vectors.S are out of sync.
const _: () = {
    if core::mem::size_of::<ExceptionFrame>() != 288 {
        panic!("ExceptionFrame size mismatch — update vectors.S offsets");
    }
};

// ---------------------------------------------------------------------------
// Exception Class (EC) — ESR_EL1 bits [31:26]
//
// Reference: ARM ARM DDI 0487 D13.2.36, Table D13-3.
// ---------------------------------------------------------------------------

/// Extract the Exception Class field from ESR_EL1.
#[inline]
pub fn esr_exception_class(esr: u64) -> u8 {
    ((esr >> 26) & 0x3F) as u8
}

/// Extract the Instruction-Specific Syndrome from ESR_EL1 bits [24:0].
#[inline]
pub fn esr_instruction_specific_syndrome(esr: u64) -> u32 {
    (esr & 0x1FF_FFFF) as u32
}

// EC value constants — ARM ARM DDI 0487 D13.2.36 Table D13-3.
pub mod ec {
    /// Unknown reason.
    pub const UNKNOWN: u8 = 0x00;
    /// Trapped WFI/WFE instruction.
    pub const WFX_TRAPPED: u8 = 0x01;
    /// Illegal execution state.
    pub const ILLEGAL_STATE: u8 = 0x0E;
    /// SVC instruction executed in AArch64.
    pub const SVC_AARCH64: u8 = 0x15;
    /// Trapped MSR/MRS/system instruction.
    pub const MSR_MRS_SYSTEM: u8 = 0x18;
    /// Instruction abort from EL0.
    pub const INSTRUCTION_ABORT_EL0: u8 = 0x20;
    /// Instruction abort from EL1 (e.g. kernel page fault).
    pub const INSTRUCTION_ABORT_EL1: u8 = 0x21;
    /// PC alignment fault.
    pub const PC_ALIGNMENT: u8 = 0x22;
    /// Data abort from EL0.
    pub const DATA_ABORT_EL0: u8 = 0x24;
    /// Data abort from EL1 (e.g. kernel data page fault).
    pub const DATA_ABORT_EL1: u8 = 0x25;
    /// SP alignment fault.
    pub const SP_ALIGNMENT: u8 = 0x26;
    /// SError interrupt.
    pub const SERROR: u8 = 0x2F;
    /// Breakpoint instruction (BRK).
    pub const BREAKPOINT: u8 = 0x3C;

    /// Human-readable name for an EC value.
    pub fn name(ec: u8) -> &'static str {
        match ec {
            UNKNOWN              => "Unknown",
            WFX_TRAPPED          => "WFI/WFE trapped",
            ILLEGAL_STATE        => "Illegal execution state",
            SVC_AARCH64          => "SVC (AArch64)",
            MSR_MRS_SYSTEM       => "MSR/MRS/System instruction",
            INSTRUCTION_ABORT_EL0 => "Instruction abort (EL0)",
            INSTRUCTION_ABORT_EL1 => "Instruction abort (EL1)",
            PC_ALIGNMENT         => "PC alignment fault",
            DATA_ABORT_EL0       => "Data abort (EL0)",
            DATA_ABORT_EL1       => "Data abort (EL1)",
            SP_ALIGNMENT         => "SP alignment fault",
            SERROR               => "SError",
            BREAKPOINT           => "Breakpoint (BRK)",
            _                    => "Reserved/unknown EC",
        }
    }
}

// ---------------------------------------------------------------------------
// Data Fault Status Code (DFSC) — ISS bits [5:0] for data/instruction aborts
//
// Reference: ARM ARM DDI 0487 D13.2.36 Table D13-5.
// ---------------------------------------------------------------------------

/// Decode the Data/Instruction Fault Status Code from the ISS field.
pub fn describe_fault_status(iss: u32) -> &'static str {
    let dfsc = iss & 0x3F; // bits [5:0]
    match dfsc & 0b111100 {
        0x00 => "Address-size fault",
        0x04 => "Translation fault",
        0x08 => "Access flag fault",
        0x0C => "Permission fault",
        0x10 => "Synchronous external abort",
        0x21 => "Alignment fault",
        _    => "Other fault",
    }
}

/// True if the ISS WnR bit [6] is set (write caused the data abort).
#[inline]
pub fn iss_is_write(iss: u32) -> bool {
    iss & (1 << 6) != 0
}

// ---------------------------------------------------------------------------
// ExceptionFrame display — used in panic/fault messages
// ---------------------------------------------------------------------------

impl fmt::Display for ExceptionFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ec  = esr_exception_class(self.esr);
        let iss = esr_instruction_specific_syndrome(self.esr);
        writeln!(formatter, "  ELR  = {:#018x}  FAR  = {:#018x}", self.elr, self.far)?;
        writeln!(formatter, "  ESR  = {:#018x}  SPSR = {:#018x}", self.esr, self.spsr)?;
        writeln!(formatter, "  EC   = {:#04x} ({})  ISS = {:#010x}", ec, ec::name(ec), iss)?;
        writeln!(formatter, "  SP   = {:#018x}", self.sp)?;
        for pair in 0..15usize {
            writeln!(
                formatter,
                "  x{:<2} = {:#018x}  x{:<2} = {:#018x}",
                pair * 2,     self.x[pair * 2],
                pair * 2 + 1, self.x[pair * 2 + 1],
            )?;
        }
        writeln!(formatter, "  x30 = {:#018x}", self.x[30])
    }
}

// ---------------------------------------------------------------------------
// Handler implementations — called from assembly stubs in vectors.S
//
// All handlers are `extern "C"` and take `*mut ExceptionFrame` as x0.
// They must not unwind — panics are caught at the top level by the panic
// handler which spins indefinitely.
// ---------------------------------------------------------------------------

/// Print an exception frame to UART and the framebuffer for post-mortem debugging.
fn print_exception(label: &str, frame: &ExceptionFrame) {
    // ── UART output (always available, even without a framebuffer) ────────────
    uart::puts("\r\n--- EXCEPTION: ");
    uart::puts(label);
    uart::puts(" ---\r\n");

    let ec  = esr_exception_class(frame.esr);
    let iss = esr_instruction_specific_syndrome(frame.esr);

    uart::puts("  EC   = ");
    uart::put_hex(ec as u64);
    uart::puts(" (");
    uart::puts(ec::name(ec));
    uart::puts(")\r\n");

    uart::puts("  ELR  = ");
    uart::put_hex(frame.elr);
    uart::puts("  FAR  = ");
    uart::put_hex(frame.far);
    uart::puts("\r\n  ESR  = ");
    uart::put_hex(frame.esr);
    uart::puts("  SPSR = ");
    uart::put_hex(frame.spsr);
    uart::puts("\r\n");

    if matches!(ec, ec::DATA_ABORT_EL0 | ec::DATA_ABORT_EL1
                  | ec::INSTRUCTION_ABORT_EL0 | ec::INSTRUCTION_ABORT_EL1) {
        uart::puts("  Fault: ");
        uart::puts(describe_fault_status(iss));
        if ec == ec::DATA_ABORT_EL0 || ec == ec::DATA_ABORT_EL1 {
            uart::puts(if iss_is_write(iss) { " (write)" } else { " (read)" });
        }
        uart::puts("\r\n");
    }

    // General-purpose register dump over UART.
    {
        use core::fmt::Write as _;
        uart::puts("  General-purpose registers:\r\n");
        for pair in 0..15usize {
            let _ = write!(
                uart::UartWriter,
                "  x{:<2} = {:#018x}  x{:<2} = {:#018x}\r\n",
                pair * 2,     frame.x[pair * 2],
                pair * 2 + 1, frame.x[pair * 2 + 1],
            );
        }
        let _ = write!(uart::UartWriter,
            "  x30 = {:#018x}  SP  = {:#018x}\r\n",
            frame.x[30], frame.sp);
    }

    // ── Framebuffer output ────────────────────────────────────────────────────
    // Render a panic screen so the crash is visible without a serial terminal.
    // Uses no heap or scheduler — safe even if they are corrupted.
    use core::fmt::Write as _;
    crate::drivers::console::console_panic_screen(label);
    let mut writer = crate::drivers::console::ConsoleWriter;
    let _ = writeln!(writer, "  EC  = {:#04x}  {}", ec, ec::name(ec));
    let _ = writeln!(writer, "  ELR = {:#018x}", frame.elr);
    let _ = writeln!(writer, "  FAR = {:#018x}", frame.far);
    let _ = writeln!(writer, "  ESR = {:#018x}  SPSR = {:#018x}", frame.esr, frame.spsr);
    if matches!(ec, ec::DATA_ABORT_EL0 | ec::DATA_ABORT_EL1
                  | ec::INSTRUCTION_ABORT_EL0 | ec::INSTRUCTION_ABORT_EL1) {
        let fault_str = describe_fault_status(iss);
        let rw = if ec == ec::DATA_ABORT_EL0 || ec == ec::DATA_ABORT_EL1 {
            if iss_is_write(iss) { " (write)" } else { " (read)" }
        } else { "" };
        let _ = writeln!(writer, "  Fault: {}{}", fault_str, rw);
    }
    let _ = writeln!(writer, "\n  General-purpose registers:");
    for pair in 0..15usize {
        let _ = writeln!(
            writer,
            "  x{:<2} = {:#018x}  x{:<2} = {:#018x}",
            pair * 2,     frame.x[pair * 2],
            pair * 2 + 1, frame.x[pair * 2 + 1],
        );
    }
    let _ = writeln!(writer, "  x30 = {:#018x}  SP  = {:#018x}", frame.x[30], frame.sp);
}

/// Halt the CPU permanently — used after unrecoverable exceptions.
#[inline(never)]
fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) };
    }
}

// --- EL1 exception handlers ---

#[no_mangle]
pub extern "C" fn exception_handler_unexpected(frame: *mut ExceptionFrame) {
    let frame = unsafe { &*frame };
    print_exception("unexpected (Group A/D or SError/FIQ)", frame);
    halt();
}

#[no_mangle]
pub extern "C" fn exception_handler_sync_el1(frame: *mut ExceptionFrame) {
    let ec = unsafe { esr_exception_class((*frame).esr) };

    match ec {
        ec::BREAKPOINT => {
            // BRK in kernel — advance ELR past the BRK instruction and continue.
            // BRK is a 4-byte instruction.
            unsafe { (*frame).elr += 4 };
        }
        _ => {
            print_exception("synchronous EL1 (kernel fault)", unsafe { &*frame });
            halt();
        }
    }
}

#[no_mangle]
pub extern "C" fn exception_handler_irq_el1(_frame: *mut ExceptionFrame) {
    // Dispatch the IRQ via the platform IRQ controller.
    crate::platform::irq_dispatch();
}

// --- EL0 exception handlers ---

#[no_mangle]
pub extern "C" fn exception_handler_sync_el0(frame: *mut ExceptionFrame) {
    let ec = unsafe { esr_exception_class((*frame).esr) };

    match ec {
        ec::SVC_AARCH64 => {
            // Extract syscall number from ESR_EL1.ISS[15:0].
            // ARM ARM DDI 0487, §D13.2.36: ISS for SVC is imm16 = bits [15:0].
            let syscall_number = unsafe { (*frame).esr & 0xFFFF };
            crate::systemcalls::dispatch(frame, syscall_number);
        }
        ec::DATA_ABORT_EL0 | ec::INSTRUCTION_ABORT_EL0 => {
            let esr = unsafe { (*frame).esr };
            let far = unsafe { (*frame).far };
            let iss = esr_instruction_specific_syndrome(esr);

            // Attempt recovery: CoW page fault or guard-page SIGSEGV delivery.
            // With IRQs re-enabled inside the handler, we mask them here first.
            // The scheduler invariant requires IRQs disabled before calling into it.
            unsafe {
                core::arch::asm!("msr daifset, #2", options(nostack, nomem));
            }

            // Ask the memory subsystem if this fault is recoverable (CoW copy).
            let recovered = unsafe { crate::memory::handle_page_fault(far, iss, ec == ec::DATA_ABORT_EL0) };

            if recovered {
                // Re-enable IRQs and return to user space.
                unsafe {
                    core::arch::asm!("msr daifclr, #2", options(nostack, nomem));
                }
                return;
            }

            // Non-recoverable fault — send SIGSEGV (signal 11) to the current process.
            let elr_at_fault = unsafe { (*frame).elr };
            let sp_at_fault  = unsafe { (*frame).sp };
            uart::puts("\r\n[fault] SIGSEGV: user-space memory fault\r\n");
            uart::puts("  ELR  = ");
            uart::put_hex(elr_at_fault);
            uart::puts("\r\n  FAR  = ");
            uart::put_hex(far);
            uart::puts("\r\n  SP   = ");
            uart::put_hex(sp_at_fault);
            uart::puts("\r\n  ESR  = ");
            uart::put_hex(esr);
            uart::puts("\r\n");

            unsafe {
                crate::scheduler::with_scheduler::<_, ()>(|scheduler| {
                    let pid = scheduler.current_pid();
                    scheduler.send_signal_to(pid, 11); // SIGSEGV
                    // Exit the process — SIGSEGV default action is terminate.
                    scheduler.exit(-11);
                });
            }
            // exit() does not return.
        }
        _ => {
            print_exception("synchronous EL0 (user fault)", unsafe { &*frame });
            halt();
        }
    }
}

#[no_mangle]
pub extern "C" fn exception_handler_irq_el0(_frame: *mut ExceptionFrame) {
    // Dispatch the IRQ via the platform IRQ controller.
    crate::platform::irq_dispatch();
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

extern "C" {
    /// Symbol exported by vectors.S — the address of the vector table.
    static exception_vectors: u8;
}

/// Install the exception vector table and unmask IRQs.
///
/// Must be called after the GIC is initialised so that no unhandled IRQ
/// arrives before the vector table is in place.
///
/// # Safety
/// Must be called from EL1 before any user process or interrupt is enabled.
pub unsafe fn exceptions_init() {
    // Write the virtual address of the vector table to VBAR_EL1.
    // ISB ensures the new VBAR is visible to all subsequent instruction fetches.
    // Reference: ARM ARM DDI 0487 D1.10.2.
    let vbar = &exception_vectors as *const u8 as u64;
    core::arch::asm!(
        "msr vbar_el1, {vbar}",
        "isb",
        vbar = in(reg) vbar,
        options(nostack, preserves_flags)
    );

    // Clear DAIF.I to unmask IRQs at EL1.
    // DAIF bits: D=debug, A=SError, I=IRQ, F=FIQ.
    // We only unmask IRQs (I bit); debug aborts and SError remain masked.
    // Reference: ARM ARM DDI 0487 D1.7.1.
    core::arch::asm!(
        "msr daifclr, #2",   // bit 1 = I (IRQ mask)
        options(nostack, preserves_flags)
    );
}
