// Bazzulto OS - ARM64 kernel entry point
// This is the first code that runs after Limine hands control to the kernel.
// Responsibilities: set up the stack, then jump to kernel_main in C.

.section .text
.global _start

_start:
    // Load the address of the top of our stack into x9 (a scratch register)
    // and move it into SP. This must happen before any C function is called.
    adr x9, stack_top
    mov sp, x9

    // Clear the frame pointer so stack traces stop here instead of
    // wandering into garbage memory above us.
    mov x29, #0

    // Jump to the C kernel entry point. x0 still holds the pointer to the
    // Limine boot info structure, as placed there by the bootloader.
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
