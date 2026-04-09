#pragma once

#include <stdint.h>

// CPU register state saved on the stack when an exception occurs.
// The exception entry code pushes all general-purpose registers before
// calling the C handler, and restores them on return.
struct exception_frame {
    uint64_t x0,  x1,  x2,  x3;
    uint64_t x4,  x5,  x6,  x7;
    uint64_t x8,  x9,  x10, x11;
    uint64_t x12, x13, x14, x15;
    uint64_t x16, x17, x18, x19;
    uint64_t x20, x21, x22, x23;
    uint64_t x24, x25, x26, x27;
    uint64_t x28, x29, x30;  // x29=frame pointer, x30=link register
    uint64_t elr;    // Exception Link Register: address of the faulting instruction
    uint64_t spsr;   // Saved Program Status Register: CPU flags at time of exception
    uint64_t esr;    // Exception Syndrome Register: encodes the exception cause
    uint64_t far;    // Fault Address Register: virtual address that caused the fault
};

// Install the exception vector table and configure VBAR_EL1.
void exceptions_init(void);

// Called from Assembly for EL1 synchronous exceptions (page faults, etc.)
void exception_handler_sync_el1(struct exception_frame *frame);

// Called from Assembly for EL1 IRQ (timer, hardware interrupts)
void exception_handler_irq_el1(struct exception_frame *frame);

// Called from Assembly for unexpected exceptions (groups A, D, FIQ, SError)
void exception_handler_unexpected(struct exception_frame *frame);
