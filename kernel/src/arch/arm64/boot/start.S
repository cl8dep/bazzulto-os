// Bazzulto OS — AArch64 kernel entry point (Rust version)
//
// This file is included via core::arch::global_asm! from src/arch/arm64/boot/mod.rs.
// It must be in .text._start so the linker script can force it to the top of .text.
//
// Limine boot contract:
//   - CPU at EL1, all interrupts masked (PSTATE.{D,A,I,F} = 1)
//   - MMU ON, TTBR1_EL1 active, HHDM mapped
//   - SPSel: undefined (rev 0) or 0 (rev 6+) — must set to 1
//   - x0-x29 = 0; x30 = 0 (must not return from entry)

.section .text._start, "ax", @progbits
.global _start

_start:
    // Select SP_EL1 as the active stack pointer.
    // ARM ARM D1.8.2: SPSel=1 → SP_EL1 (EL1h mode).
    // Required because exception entry at EL1 forces PSTATE.SP=1 (ARM ARM D1.10.1).
    msr spsel, #1

    // Set kernel stack. ADRP+ADD covers the full ±4 GB PC-relative range,
    // needed because embedded ELF blobs in .rodata may push .bss beyond ±1 MB.
    adrp x9, stack_top
    add  x9, x9, :lo12:stack_top
    mov  sp, x9

    // Clear frame pointer so stack traces terminate here.
    mov x29, #0

    // Call Rust kernel_main (extern "C" fn).
    bl kernel_main

    // kernel_main must not return. Halt permanently if it does.
.Lhalt:
    wfe
    b .Lhalt


// 16 KB kernel stack in BSS (zero-initialized, not in ELF file).
.section .bss
.align 4        // 2^4 = 16-byte alignment — required by AAPCS64
stack_bottom:
    .space 16384
stack_top:
