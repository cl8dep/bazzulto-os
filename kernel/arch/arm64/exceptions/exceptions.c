#include "../../../../include/bazzulto/exceptions.h"
#include "../../../../include/bazzulto/console.h"

// Defined in exception_vectors.S — the Assembly table we install.
extern void exception_vectors(void);

// ESR_EL1 exception class field: bits [31:26]
#define ESR_EC_SHIFT  26
#define ESR_EC_MASK   0x3F
#define ESR_EC(esr)   (((esr) >> ESR_EC_SHIFT) & ESR_EC_MASK)

// Exception classes we handle explicitly
#define EC_DATA_ABORT_EL1   0x25  // page fault on data access from EL1
#define EC_INSN_ABORT_EL1   0x21  // page fault on instruction fetch from EL1
#define EC_SVC_AARCH64      0x15  // syscall (SVC instruction from AArch64)

static void print_exception_frame(struct exception_frame *frame) {
    console_print("  ELR=0x"); // instruction that faulted
    // Print hex — temporary until we have console_printf
    char buf[17];
    uint64_t val = frame->elr;
    for (int i = 15; i >= 0; i--) {
        buf[i] = "0123456789ABCDEF"[val & 0xF];
        val >>= 4;
    }
    buf[16] = '\0';
    console_print(buf);

    console_print("  FAR=0x");
    val = frame->far;
    for (int i = 15; i >= 0; i--) {
        buf[i] = "0123456789ABCDEF"[val & 0xF];
        val >>= 4;
    }
    buf[16] = '\0';
    console_print(buf);

    console_print("  ESR=0x");
    val = frame->esr;
    for (int i = 7; i >= 0; i--) {
        buf[i] = "0123456789ABCDEF"[val & 0xF];
        val >>= 4;
    }
    buf[8] = '\0';
    console_println(buf);
}

void exception_handler_sync_el1(struct exception_frame *frame) {
    uint32_t ec = ESR_EC(frame->esr);

    switch (ec) {
        case EC_DATA_ABORT_EL1:
            console_println("KERNEL PANIC: data abort (page fault on data access)");
            break;
        case EC_INSN_ABORT_EL1:
            console_println("KERNEL PANIC: instruction abort (page fault on fetch)");
            break;
        case EC_SVC_AARCH64:
            // Syscall from kernel — not expected, but not fatal yet
            console_println("WARNING: SVC from EL1 (unexpected syscall in kernel)");
            return;
        default:
            console_println("KERNEL PANIC: unhandled synchronous exception");
            break;
    }

    print_exception_frame(frame);
    for (;;) __asm__("wfe");
}

void exception_handler_irq_el1(struct exception_frame *frame) {
    // IRQ handler stub — hardware interrupts land here.
    // For now we acknowledge and ignore; the timer and GIC will be
    // configured when the scheduler is implemented.
    (void)frame;
}

void exception_handler_unexpected(struct exception_frame *frame) {
    console_println("KERNEL PANIC: unexpected exception (group A, D, FIQ, or SError)");
    print_exception_frame(frame);
    for (;;) __asm__("wfe");
}

void exceptions_init(void) {
    __asm__ volatile(
        "msr vbar_el1, %0\n"  // Install our vector table
        "isb\n"
        :
        : "r"(exception_vectors)
        : "memory"
    );
    console_println("Exceptions: ok");
}
