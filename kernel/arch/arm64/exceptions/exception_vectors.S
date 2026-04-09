// Bazzulto OS - ARM64 Exception Vector Table
//
// The table must be aligned to 2KB (0x800) and each of the 16 entries
// occupies exactly 128 bytes (32 instructions). The CPU uses VBAR_EL1
// + offset to find the right entry.
//
// Layout:
//   0x000–0x180  Group A: EL1 with SP_EL0 (unreachable in Bazzulto)
//   0x200–0x380  Group B: EL1 with SP_EL1 (kernel exceptions — active)
//   0x400–0x580  Group C: from EL0 AArch64 (user space — future)
//   0x600–0x780  Group D: from EL0 AArch32 (32-bit compat — unused)

// Save all general-purpose registers plus exception context registers
// onto the current stack. Leaves a pointer to the frame in x0.
.macro save_exception_frame
    sub  sp, sp, #272          // 34 registers × 8 bytes
    stp  x0,  x1,  [sp, #0]
    stp  x2,  x3,  [sp, #16]
    stp  x4,  x5,  [sp, #32]
    stp  x6,  x7,  [sp, #48]
    stp  x8,  x9,  [sp, #64]
    stp  x10, x11, [sp, #80]
    stp  x12, x13, [sp, #96]
    stp  x14, x15, [sp, #112]
    stp  x16, x17, [sp, #128]
    stp  x18, x19, [sp, #144]
    stp  x20, x21, [sp, #160]
    stp  x22, x23, [sp, #176]
    stp  x24, x25, [sp, #192]
    stp  x26, x27, [sp, #208]
    stp  x28, x29, [sp, #224]
    str  x30,      [sp, #240]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #248]
    stp  x11, x12, [sp, #264]
    mov  x0, sp                // first argument to C handler = pointer to frame
.endm

// Restore all registers and return from exception.
.macro restore_exception_frame
    ldp  x9,  x10, [sp, #248]
    msr  elr_el1,  x9
    msr  spsr_el1, x10
    ldp  x0,  x1,  [sp, #0]
    ldp  x2,  x3,  [sp, #16]
    ldp  x4,  x5,  [sp, #32]
    ldp  x6,  x7,  [sp, #48]
    ldp  x8,  x9,  [sp, #64]
    ldp  x10, x11, [sp, #80]
    ldp  x12, x13, [sp, #96]
    ldp  x14, x15, [sp, #112]
    ldp  x16, x17, [sp, #128]
    ldp  x18, x19, [sp, #144]
    ldp  x20, x21, [sp, #160]
    ldp  x22, x23, [sp, #176]
    ldp  x24, x25, [sp, #192]
    ldp  x26, x27, [sp, #208]
    ldp  x28, x29, [sp, #224]
    ldr  x30,      [sp, #240]
    add  sp, sp, #272
    eret                       // return from exception: restores PC and PSTATE
.endm

// Each entry is exactly 128 bytes. We pad with NOPs to fill the space.
.macro vector_entry label
    .align 7                   // 2^7 = 128-byte alignment for each entry
    \label:
.endm

// -----------------------------------------------------------------------

.section .text
.global exception_vectors
.align 11                      // 2^11 = 2048-byte alignment for the whole table

exception_vectors:

// --- Group A: EL1 / SP_EL0 (should never happen in Bazzulto) ---

vector_entry el1_sp0_sync
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el1_sp0_irq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el1_sp0_fiq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el1_sp0_serror
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

// --- Group B: EL1 / SP_EL1 (kernel exceptions — these matter now) ---

vector_entry el1_sync
    save_exception_frame
    bl exception_handler_sync_el1
    restore_exception_frame

vector_entry el1_irq
    save_exception_frame
    bl exception_handler_irq_el1
    restore_exception_frame

vector_entry el1_fiq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el1_serror
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

// --- Group C: from EL0 AArch64 (user space — handlers to be added later) ---

vector_entry el0_64_sync
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_64_irq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_64_fiq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_64_serror
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

// --- Group D: from EL0 AArch32 (32-bit compat — not supported) ---

vector_entry el0_32_sync
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_32_irq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_32_fiq
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

vector_entry el0_32_serror
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame
