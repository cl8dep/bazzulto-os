// Bazzulto OS - ARM64 context switch
//
// void context_switch(cpu_context_t *from, cpu_context_t *to)
//
// Saves the callee-saved registers of the current process into `from`,
// then restores the registers of the next process from `to` and returns
// into it — which resumes execution at the point where `to` last called
// context_switch (or at its entry point if it has never run).
//
// We only save callee-saved registers (x19-x30) because the C calling
// convention guarantees that x0-x18 are already saved by the caller if
// they matter. The compiler takes care of that automatically.
//
// IRQ handling:
//   - RESUMED processes: context_switch is called from inside an IRQ handler
//     which has PSTATE.I=1. ret returns to scheduler_tick, which unwinds
//     through the exception handler to eret. The eret atomically restores
//     SPSR_EL1 (which had I=0) so IRQs are re-enabled safely. We do NOT
//     unmask IRQs here — doing so creates a window for nested timer IRQs
//     that steal the incoming process's timeslice.
//
//   - FRESH processes: x30 = process_entry_trampoline (see below), which
//     unmasks IRQs just before jumping to the real entry point.

.section .text
.global context_switch

context_switch:
    // Save callee-saved registers of the outgoing process into `from` (x0)
    stp  x19, x20, [x0, #0]
    stp  x21, x22, [x0, #16]
    stp  x23, x24, [x0, #32]
    stp  x25, x26, [x0, #48]
    stp  x27, x28, [x0, #64]
    stp  x29, x30, [x0, #80]
    mov  x9,  sp
    str  x9,       [x0, #96]

    // Restore callee-saved registers of the incoming process from `to` (x1)
    ldp  x19, x20, [x1, #0]
    ldp  x21, x22, [x1, #16]
    ldp  x23, x24, [x1, #32]
    ldp  x25, x26, [x1, #48]
    ldp  x27, x28, [x1, #64]
    ldp  x29, x30, [x1, #80]
    ldr  x9,       [x1, #96]
    mov  sp,  x9

    ret

// Entry trampoline for KERNEL threads that have never run before.
// scheduler_create_process sets: x30 = process_entry_trampoline,
//                                x19 = real entry point.
// context_switch ret → here → enable IRQs → jump to entry.
.global process_entry_trampoline
process_entry_trampoline:
    msr  daifclr, #2   // unmask IRQs (the process must be preemptible)
    br   x19            // jump to the real entry point (saved in callee-saved x19)

// Entry trampoline for USER processes (EL0).
// scheduler_create_user_process sets: x30 = process_entry_trampoline_user,
//                                     x19 = user entry point VA,
//                                     x20 = user stack top VA.
// context_switch ret → here → set ELR/SPSR/SP_EL0 → eret to EL0.
.global process_entry_trampoline_user
process_entry_trampoline_user:
    msr  elr_el1, x19         // return address = user entry point
    msr  spsr_el1, xzr        // SPSR = 0 = EL0t, IRQs unmasked, flags clear
    msr  sp_el0, x20           // set user stack pointer

    // Zero all general-purpose registers to prevent kernel data leakage.
    // After eret, the user sees only zeros in x0-x30.
    mov  x0,  #0
    mov  x1,  #0
    mov  x2,  #0
    mov  x3,  #0
    mov  x4,  #0
    mov  x5,  #0
    mov  x6,  #0
    mov  x7,  #0
    mov  x8,  #0
    mov  x9,  #0
    mov  x10, #0
    mov  x11, #0
    mov  x12, #0
    mov  x13, #0
    mov  x14, #0
    mov  x15, #0
    mov  x16, #0
    mov  x17, #0
    mov  x18, #0
    mov  x19, #0
    mov  x20, #0
    mov  x21, #0
    mov  x22, #0
    mov  x23, #0
    mov  x24, #0
    mov  x25, #0
    mov  x26, #0
    mov  x27, #0
    mov  x28, #0
    mov  x29, #0
    mov  x30, #0
    eret                       // → EL0: PC=ELR, PSTATE=SPSR
