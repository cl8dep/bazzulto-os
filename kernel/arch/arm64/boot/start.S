// Bazzulto OS - ARM64 kernel entry point
// This is the first code that runs after Limine hands control to the kernel.
//
// Limine boot contract (limine-protocol PROTOCOL.md, verified from trunk):
//   - CPU at EL1, all interrupts masked (PSTATE.{D,A,I,F} = 1)
//   - MMU ON: SCTLR_EL1.{M,C,I} = 1 (MMU, D-cache, I-cache enabled)
//   - MAIR_EL1: Attr0=0xFF (normal WB-RWA), Attr1=framebuffer type
//   - TCR_EL1: T0SZ=T1SZ=16, 4KB granule, Inner Shareable, WB-RWA
//   - TTBR1_EL1: bootloader page tables (kernel + HHDM mapped)
//   - VBAR_EL1: bootloader vectors that just hang — we must replace
//   - SPSel: UNDEFINED (rev 0) or 0 (rev 6+) — we must set to 1
//   - SP: bootloader stack (64KB default) — we override with our own
//   - x0-x28, x29 = 0; x30 = 0 (must not return from entry)
//   - GIC: firmware-dependent (not initialized by Limine)

.section .text
.global _start

_start:
    // Select SP_EL1 as the stack pointer for EL1.
    //
    // ARM ARM D1.8.2: SPSel controls which SP is used at the current EL.
    //   SPSel=0 → SP_EL0 (EL1t mode)
    //   SPSel=1 → SP_EL1 (EL1h mode)
    //
    // We MUST use SP_EL1 because exception entry at EL1 forces PSTATE.SP=1
    // (ARM ARM D1.10.1). If we ran with SP_EL0, exception handlers would
    // use the uninitialized SP_EL1 and crash immediately.
    msr spsel, #1

    // Set up the kernel stack. Now that SPSel=1, `mov sp` writes SP_EL1.
    adr x9, stack_top
    mov sp, x9

    // Clear the frame pointer so stack traces stop here.
    mov x29, #0

    // Jump to the C kernel entry point.
    bl kernel_main

    // kernel_main should never return. If it does, halt the CPU permanently.
halt:
    wfe         // Wait For Event: puts the CPU in a low-power sleep state
    b halt      // Loop back in case of a spurious wakeup


// Reserve 16KB for the kernel stack in the BSS segment.
// BSS is zero-initialized and not stored in the ELF file, just reserved.
.section .bss
.align 4        // 2^4 = 16-byte alignment, required by AAPCS64 calling convention
stack_bottom:
    .space 16384
stack_top:
