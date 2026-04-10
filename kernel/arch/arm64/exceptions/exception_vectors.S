// Bazzulto OS - ARM64 Exception Vector Table
//
// ARM64 requires each of the 16 entries to fit within 128 bytes (32 instructions)
// at fixed offsets from VBAR_EL1. Since our save+restore frame is ~184 bytes,
// we can't inline it. Each entry is a single branch to a handler outside the table.
//
// Layout (offsets from VBAR_EL1):
//   0x000–0x180  Group A: EL1 with SP_EL0 (unreachable in Bazzulto — we use SPSel=1)
//   0x200–0x380  Group B: EL1 with SP_EL1 (kernel exceptions — active)
//   0x400–0x580  Group C: from EL0 AArch64 (user space exceptions)
//   0x600–0x780  Group D: from EL0 AArch32 (32-bit compat — unused)

// -----------------------------------------------------------------------
// save_exception_frame: for exceptions taken from EL1 (Groups A, B)
//
// Saves all GPRs + system registers onto SP_EL1 (the current kernel stack).
// The SP field at offset 248 stores the kernel stack pointer BEFORE the
// frame was pushed (add x9, sp, #288).
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
// -----------------------------------------------------------------------
.macro save_exception_frame
    sub  sp, sp, #288
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
    add  x9,  sp, #288
    str  x9,       [sp, #248]      // SP = kernel SP before frame push
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #256]
    stp  x11, x12, [sp, #272]
    mov  x0, sp
.endm

// -----------------------------------------------------------------------
// save_exception_frame_el0: for exceptions taken from EL0 (Group C)
//
// Identical to save_exception_frame except the SP field stores SP_EL0
// (the user stack pointer) instead of the kernel SP. When the CPU takes
// an exception from EL0, it automatically switches to SP_EL1 (the kernel
// stack). SP_EL0 is preserved and must be read explicitly.
// ARM ARM D1.10.1: on exception entry, PSTATE.SP is set to 1 (SP_ELx).
// -----------------------------------------------------------------------
.macro save_exception_frame_el0
    sub  sp, sp, #288
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
    mrs  x9,  sp_el0
    str  x9,       [sp, #248]      // SP = user SP (SP_EL0)
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #256]
    stp  x11, x12, [sp, #272]
    mov  x0, sp
.endm

// Restore all registers and return from exception.
// Works for both EL1→EL1 and EL1→EL0 returns: eret atomically restores
// PC from ELR_EL1 and PSTATE from SPSR_EL1 (including the target EL).
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
    eret
.endm

// -----------------------------------------------------------------------
// Exception Vector Table
// Each entry MUST fit within 128 bytes (.align 7 = 128-byte boundary).
// -----------------------------------------------------------------------

.section .text
.global exception_vectors
.align 11                      // 2^11 = 2048-byte alignment for the whole table

exception_vectors:

// --- Group A: EL1 / SP_EL0 (unreachable — we use SPSel=1) ---
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected
.align 7
    b __handle_unexpected

// --- Group B: EL1 / SP_EL1 (kernel exceptions) ---
.align 7
    b __handle_sync_el1
.align 7
    b __handle_irq_el1
.align 7
    b __handle_unexpected      // FIQ — unused
.align 7
    b __handle_unexpected      // SError — unexpected

// --- Group C: from EL0 AArch64 (user space exceptions) ---
.align 7
    b __handle_sync_el0
.align 7
    b __handle_irq_el0
.align 7
    b __handle_unexpected      // FIQ — unused
.align 7
    b __handle_unexpected      // SError — unexpected

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
// Handler implementations
// -----------------------------------------------------------------------

__handle_unexpected:
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

// EL1 handlers (kernel exceptions)
__handle_sync_el1:
    save_exception_frame
    bl exception_handler_sync_el1
    restore_exception_frame

__handle_irq_el1:
    save_exception_frame
    bl exception_handler_irq_el1
    restore_exception_frame

// EL0 handlers (user space exceptions) — use save_exception_frame_el0
// to capture SP_EL0 (user stack) instead of SP_EL1 (kernel stack).
__handle_sync_el0:
    save_exception_frame_el0
    bl exception_handler_sync_el0
    restore_exception_frame

__handle_irq_el0:
    save_exception_frame_el0
    bl exception_handler_irq_el0
    restore_exception_frame
