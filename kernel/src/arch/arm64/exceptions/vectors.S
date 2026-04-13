// arch/arm64/exceptions/vectors.S — AArch64 exception vector table.
//
// ARM64 requires the vector table to be aligned to 2048 bytes (2^11).
// Each of the 16 entries occupies a fixed 128-byte (2^7) slot.
// Because the full save+restore frame is ~300 bytes, we cannot inline it;
// each slot is a single branch to a handler defined below the table.
//
// Table layout (offsets from VBAR_EL1):
//   0x000–0x17F  Group A: EL1 / SP_EL0  — unreachable (we always use SPSel=1)
//   0x200–0x37F  Group B: EL1 / SP_EL1  — kernel exceptions (active)
//   0x400–0x57F  Group C: EL0 / AArch64 — user-space exceptions (active)
//   0x600–0x77F  Group D: EL0 / AArch32 — 32-bit compat (unsupported)
//
// Reference: ARM ARM DDI 0487 D1.10.2 "Exception vectors".

// ---------------------------------------------------------------------------
// Exception frame layout (36 × 8 = 288 bytes)
//
// The layout below must match ExceptionFrame in exceptions/mod.rs exactly.
// A compile-time assertion in Rust verifies the size.
//
//   Offset  Contents
//   0x000   x0 – x1
//   0x010   x2 – x3
//   0x020   x4 – x5
//   0x030   x6 – x7
//   0x040   x8 – x9
//   0x050   x10– x11
//   0x060   x12– x13
//   0x070   x14– x15
//   0x080   x16– x17
//   0x090   x18– x19
//   0x0A0   x20– x21
//   0x0B0   x22– x23
//   0x0C0   x24– x25
//   0x0D0   x26– x27
//   0x0E0   x28– x29
//   0x0F0   x30 (link register / LR)
//   0x0F8   SP  (SP_EL1 for EL1 exceptions; SP_EL0 for EL0 exceptions)
//   0x100   ELR_EL1  (program counter at exception entry)
//   0x108   SPSR_EL1 (processor state at exception entry)
//   0x110   ESR_EL1  (exception syndrome)
//   0x118   FAR_EL1  (fault address)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// save_exception_frame — for exceptions taken from EL1 (Groups A and B)
//
// SP at offset 0x0F8 stores SP_EL1 *before* the frame was pushed, i.e. the
// kernel stack pointer that was active at the moment the exception fired.
// Reference: ARM ARM DDI 0487 D1.10.1 (exception stack pointer selection).
// ---------------------------------------------------------------------------
.macro save_exception_frame
    sub  sp,  sp,  #288
    stp  x0,  x1,  [sp, #0x000]
    stp  x2,  x3,  [sp, #0x010]
    stp  x4,  x5,  [sp, #0x020]
    stp  x6,  x7,  [sp, #0x030]
    stp  x8,  x9,  [sp, #0x040]
    stp  x10, x11, [sp, #0x050]
    stp  x12, x13, [sp, #0x060]
    stp  x14, x15, [sp, #0x070]
    stp  x16, x17, [sp, #0x080]
    stp  x18, x19, [sp, #0x090]
    stp  x20, x21, [sp, #0x0A0]
    stp  x22, x23, [sp, #0x0B0]
    stp  x24, x25, [sp, #0x0C0]
    stp  x26, x27, [sp, #0x0D0]
    stp  x28, x29, [sp, #0x0E0]
    str  x30,      [sp, #0x0F0]
    // SP_EL1 before frame push = current SP + 288.
    add  x9,  sp,  #288
    str  x9,       [sp, #0x0F8]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #0x100]
    stp  x11, x12, [sp, #0x110]
    // Pass frame pointer as first argument (x0) to the C/Rust handler.
    mov  x0,  sp
.endm

// ---------------------------------------------------------------------------
// save_exception_frame_el0 — for exceptions taken from EL0 (Group C)
//
// Identical to save_exception_frame except SP at offset 0x0F8 stores SP_EL0
// (the user-space stack pointer).  When the CPU takes an exception from EL0,
// it switches to SP_EL1 automatically; SP_EL0 is preserved and must be read
// via `mrs x9, sp_el0`.
// Reference: ARM ARM DDI 0487 D1.10.1.
// ---------------------------------------------------------------------------
.macro save_exception_frame_el0
    sub  sp,  sp,  #288
    stp  x0,  x1,  [sp, #0x000]
    stp  x2,  x3,  [sp, #0x010]
    stp  x4,  x5,  [sp, #0x020]
    stp  x6,  x7,  [sp, #0x030]
    stp  x8,  x9,  [sp, #0x040]
    stp  x10, x11, [sp, #0x050]
    stp  x12, x13, [sp, #0x060]
    stp  x14, x15, [sp, #0x070]
    stp  x16, x17, [sp, #0x080]
    stp  x18, x19, [sp, #0x090]
    stp  x20, x21, [sp, #0x0A0]
    stp  x22, x23, [sp, #0x0B0]
    stp  x24, x25, [sp, #0x0C0]
    stp  x26, x27, [sp, #0x0D0]
    stp  x28, x29, [sp, #0x0E0]
    str  x30,      [sp, #0x0F0]
    // SP_EL0 = user stack pointer at the moment of exception.
    mrs  x9,  sp_el0
    str  x9,       [sp, #0x0F8]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, esr_el1
    mrs  x12, far_el1
    stp  x9,  x10, [sp, #0x100]
    stp  x11, x12, [sp, #0x110]
    mov  x0,  sp
.endm

// ---------------------------------------------------------------------------
// restore_exception_frame — for return from EL1→EL1 exception
// ---------------------------------------------------------------------------
.macro restore_exception_frame
    ldp  x9,  x10, [sp, #0x100]
    msr  elr_el1,  x9
    msr  spsr_el1, x10
    ldp  x0,  x1,  [sp, #0x000]
    ldp  x2,  x3,  [sp, #0x010]
    ldp  x4,  x5,  [sp, #0x020]
    ldp  x6,  x7,  [sp, #0x030]
    ldp  x8,  x9,  [sp, #0x040]
    ldp  x10, x11, [sp, #0x050]
    ldp  x12, x13, [sp, #0x060]
    ldp  x14, x15, [sp, #0x070]
    ldp  x16, x17, [sp, #0x080]
    ldp  x18, x19, [sp, #0x090]
    ldp  x20, x21, [sp, #0x0A0]
    ldp  x22, x23, [sp, #0x0B0]
    ldp  x24, x25, [sp, #0x0C0]
    ldp  x26, x27, [sp, #0x0D0]
    ldp  x28, x29, [sp, #0x0E0]
    ldr  x30,      [sp, #0x0F0]
    add  sp,  sp,  #288
    eret
.endm

// ---------------------------------------------------------------------------
// restore_exception_frame_el0 — for return from EL1→EL0 exception
//
// Must also restore SP_EL0 before touching any GPRs that might have changed
// during a context switch.  SP_EL0 is written first (while x9 is still safe
// to use as a scratch register) so that subsequent GPR loads cannot clobber it.
// Reference: ARM ARM DDI 0487 D1.10.1.
// ---------------------------------------------------------------------------
.macro restore_exception_frame_el0
    ldp  x9,  x10, [sp, #0x100]
    msr  elr_el1,  x9
    msr  spsr_el1, x10
    // Restore SP_EL0 BEFORE any GPR loads overwrite x9.
    ldr  x9,       [sp, #0x0F8]
    msr  sp_el0,   x9
    ldp  x0,  x1,  [sp, #0x000]
    ldp  x2,  x3,  [sp, #0x010]
    ldp  x4,  x5,  [sp, #0x020]
    ldp  x6,  x7,  [sp, #0x030]
    ldp  x8,  x9,  [sp, #0x040]     // x9 overwritten — sp_el0 already written above
    ldp  x10, x11, [sp, #0x050]
    ldp  x12, x13, [sp, #0x060]
    ldp  x14, x15, [sp, #0x070]
    ldp  x16, x17, [sp, #0x080]
    ldp  x18, x19, [sp, #0x090]
    ldp  x20, x21, [sp, #0x0A0]
    ldp  x22, x23, [sp, #0x0B0]
    ldp  x24, x25, [sp, #0x0C0]
    ldp  x26, x27, [sp, #0x0D0]
    ldp  x28, x29, [sp, #0x0E0]
    ldr  x30,      [sp, #0x0F0]
    add  sp,  sp,  #288
    eret
.endm

// ---------------------------------------------------------------------------
// Exception Vector Table
// ---------------------------------------------------------------------------

.section .text.exception_vectors, "ax", @progbits
.global exception_vectors
.align 11                             // 2^11 = 2048-byte table alignment

exception_vectors:

// --- Group A: EL1 / SP_EL0 (unreachable — Bazzulto always uses SPSel=1) ---
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected

// --- Group B: EL1 / SP_EL1 (kernel exceptions — active) ---
.align 7
    b __exception_sync_el1
.align 7
    b __exception_irq_el1
.align 7
    b __exception_unexpected           // FIQ — not used
.align 7
    b __exception_unexpected           // SError — not expected

// --- Group C: EL0 / AArch64 (user-space exceptions — active) ---
.align 7
    b __exception_sync_el0
.align 7
    b __exception_irq_el0
.align 7
    b __exception_unexpected           // FIQ — not used
.align 7
    b __exception_unexpected           // SError — not expected

// --- Group D: EL0 / AArch32 (32-bit compat — not supported) ---
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected
.align 7
    b __exception_unexpected

// ---------------------------------------------------------------------------
// Handler stubs — save frame, call Rust handler, restore frame.
//
// The Rust functions are declared `extern "C" fn` and take `*mut ExceptionFrame`.
// The frame pointer is passed in x0 by the `save_exception_frame` macro.
// ---------------------------------------------------------------------------

__exception_unexpected:
    save_exception_frame
    bl exception_handler_unexpected
    restore_exception_frame

__exception_sync_el1:
    save_exception_frame
    bl exception_handler_sync_el1
    restore_exception_frame

__exception_irq_el1:
    save_exception_frame
    bl exception_handler_irq_el1
    restore_exception_frame

__exception_sync_el0:
    save_exception_frame_el0
    bl exception_handler_sync_el0
    restore_exception_frame_el0

__exception_irq_el0:
    save_exception_frame_el0
    bl exception_handler_irq_el0
    restore_exception_frame_el0

// ---------------------------------------------------------------------------
// fork_child_resume — entry point for newly forked child processes.
//
// When the scheduler first context-switches to a forked child, it restores SP
// to the bottom of the copied ExceptionFrame on the child's kernel stack and
// branches here (via the saved x30 in the context block).
//
// SP therefore points to a fully populated ExceptionFrame with x0 already
// set to 0 (the child's fork() return value).  restore_exception_frame_el0
// restores all registers including SP_EL0 and ELR_EL1, then erets to EL0.
// ---------------------------------------------------------------------------
.global fork_child_resume
fork_child_resume:
    restore_exception_frame_el0
