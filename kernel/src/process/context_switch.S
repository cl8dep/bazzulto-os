// process/context_switch.S — AArch64 kernel context switch and EL0 entry trampoline.
//
// Exports:
//   context_switch(from: *mut CpuContext, to: *const CpuContext)
//   process_entry_trampoline_el0
//
// CpuContext layout (must match process/mod.rs exactly — 104 bytes):
//
//   Offset  Field
//   0x000   x19
//   0x008   x20
//   0x010   x21
//   0x018   x22
//   0x020   x23
//   0x028   x24
//   0x030   x25
//   0x038   x26
//   0x040   x27
//   0x048   x28
//   0x050   x29  (frame pointer)
//   0x058   x30  (link register)
//   0x060   sp
//
// Reference: AAPCS64 §6.1.1 — x19–x28 are callee-saved; x29/x30 and SP are
// caller-visible saved state across function boundaries.
// ARM ARM DDI 0487 C6.2.339 (STP/LDP), C6.2.196 (MSR), C6.2.169 (MRS).

.section .text, "ax"

// ---------------------------------------------------------------------------
// context_switch — save caller-saved (callee-saved from ABI perspective)
//                  registers of `from`, restore them from `to`, return into
//                  the `to` task's x30.
//
// Calling convention (AAPCS64):
//   x0 = from: *mut CpuContext
//   x1 = to:   *const CpuContext
//
// On return the CPU is running in the context of `to`.  The return address
// is whatever x30 was saved in `to->link_register`.
// ---------------------------------------------------------------------------
.global context_switch
.type   context_switch, %function
context_switch:
    // --- Save the `from` context ---
    stp  x19, x20, [x0, #0x000]
    stp  x21, x22, [x0, #0x010]
    stp  x23, x24, [x0, #0x020]
    stp  x25, x26, [x0, #0x030]
    stp  x27, x28, [x0, #0x040]
    stp  x29, x30, [x0, #0x050]
    mov  x9,  sp
    str  x9,       [x0, #0x060]

    // --- Restore the `to` context ---
    ldp  x19, x20, [x1, #0x000]
    ldp  x21, x22, [x1, #0x010]
    ldp  x23, x24, [x1, #0x020]
    ldp  x25, x26, [x1, #0x030]
    ldp  x27, x28, [x1, #0x040]
    ldp  x29, x30, [x1, #0x050]
    ldr  x9,       [x1, #0x060]
    mov  sp,  x9

    // Return into x30 of `to` (its saved link register).
    ret

// ---------------------------------------------------------------------------
// process_entry_trampoline_el0 — restore an ExceptionFrame from the kernel
//                                stack and eret to EL0.
//
// Called when a newly created (fork/exec/spawn) process is first scheduled.
// The scheduler places a pointer to the process's initial ExceptionFrame at
// the top of the kernel stack (sp = frame pointer).
//
// ExceptionFrame layout (288 bytes — must match vectors.S and exceptions/mod.rs):
//
//   Offset  Field
//   0x000   x0–x1
//   ...
//   0x0E0   x28–x29
//   0x0F0   x30 (LR)
//   0x0F8   SP_EL0  (user stack pointer)
//   0x100   ELR_EL1 (user PC to return to)
//   0x108   SPSR_EL1
//   0x110   ESR_EL1 (not restored — read-only system register)
//   0x118   FAR_EL1 (not restored)
//
// We restore: ELR_EL1, SPSR_EL1, SP_EL0, x0–x30.
// Then `add sp, sp, #288` + `eret`.
//
// Reference: ARM ARM DDI 0487 D1.10.1 (exception return), C6.2.87 (ERET).
// ---------------------------------------------------------------------------
.global process_entry_trampoline_el0
.type   process_entry_trampoline_el0, %function
process_entry_trampoline_el0:
    // Restore ELR_EL1 and SPSR_EL1 first (use x9/x10 as scratch — not yet restored).
    ldp  x9,  x10, [sp, #0x100]
    msr  elr_el1,  x9
    msr  spsr_el1, x10

    // Restore SP_EL0 (user stack pointer) via x9 scratch.
    ldr  x9,       [sp, #0x0F8]
    msr  sp_el0,   x9

    // Restore all general-purpose registers x0–x30.
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

    // Discard the frame — eret will switch to EL0 with SPSR/ELR values set above.
    add  sp,  sp,  #288
    eret
