// Bazzulto OS - ARM64 Exception Vector Table
//
// ARM64 requires each of the 16 entries to fit within 128 bytes (32 instructions)
// at fixed offsets from VBAR_EL1. Since our save+restore frame is ~184 bytes,
// we can't inline it. Each entry is a single branch to a handler outside the table.
//
// Layout (offsets from VBAR_EL1):
//   0x000–0x180  Group A: EL1 with SP_EL0 (unreachable in Bazzulto)
//   0x200–0x380  Group B: EL1 with SP_EL1 (kernel exceptions — active)
//   0x400–0x580  Group C: from EL0 AArch64 (user space — future)
//   0x600–0x780  Group D: from EL0 AArch32 (32-bit compat — unused)

// Save all general-purpose registers plus exception context onto the stack.
// Leaves a pointer to the frame in x0.
//
// Frame layout (36 × 8 = 288 bytes, matches struct exception_frame):
//   [sp, #0]   x0-x1     [sp, #192] x24-x25
//   [sp, #16]  x2-x3     [sp, #208] x26-x27
//   [sp, #32]  x4-x5     [sp, #224] x28-x29
//   [sp, #48]  x6-x7     [sp, #240] x30
//   [sp, #64]  x8-x9     [sp, #248] SP (pre-exception)
//   [sp, #80]  x10-x11   [sp, #256] ELR_EL1, SPSR_EL1
//   [sp, #96]  x12-x13   [sp, #272] ESR_EL1, FAR_EL1
//   [sp, #112] x14-x15
//   [sp, #128] x16-x17
//   [sp, #144] x18-x19
//   [sp, #160] x20-x21
//   [sp, #176] x22-x23
.macro save_exception_frame
    sub  sp, sp, #288          // 36 fields × 8 = 288 bytes (16-byte aligned)
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
    // Compute and save the stack pointer as it was BEFORE this frame.
    add  x9,  sp, #288
    str  x9,       [sp, #248]
    // Save exception-specific system registers.
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #256]
    stp  x11, x12, [sp, #272]
    mov  x0, sp                // first argument to C handler = pointer to frame
.endm

// Restore all registers and return from exception.
.macro restore_exception_frame
    ldp  x9,  x10, [sp, #256]
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
    add  sp, sp, #288
    eret                       // return from exception: restores PC and PSTATE
.endm

// -----------------------------------------------------------------------
// Exception Vector Table
//
// Each entry MUST fit within 128 bytes (.align 7 = 128-byte boundary).
// We use a single branch instruction per entry — the handler code lives
// outside the table in the section below.
// -----------------------------------------------------------------------

.section .text
.global exception_vectors
.align 11                      // 2^11 = 2048-byte alignment for the whole table

exception_vectors:

// --- Group A: EL1 / SP_EL0 (should never happen in Bazzulto) ---
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected

// --- Group B: EL1 / SP_EL1 (kernel exceptions — these matter now) ---
.align 7
    b __handle_sync_el1
.align 7
    b __handle_irq_el1
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected

// --- Group C: from EL0 AArch64 (user space — handlers to be added later) ---
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected

// --- Group D: from EL0 AArch32 (32-bit compat — not supported) ---
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected

// -----------------------------------------------------------------------
// Handler implementations — defined outside the table so they can be
// any size without breaking the 128-byte-per-entry constraint.
// -----------------------------------------------------------------------

__handle_unexpected:
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

__handle_sync_el1:
    save_exception_frame
    bl exception_handler_sync_el1
    restore_exception_frame

__handle_irq_el1:
    save_exception_frame
    bl exception_handler_irq_el1
    restore_exception_frame
