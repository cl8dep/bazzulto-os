#include "../../../../include/bazzulto/exceptions.h"
#include <stdio.h>
#include "../../../../include/bazzulto/console.h"
#include "../../../../include/bazzulto/gic.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/pid.h"
#include "../../../../include/bazzulto/timer.h"
#include "../../../../include/bazzulto/uart.h"
#include "../../../../include/bazzulto/systemcall.h"
#include "../../../../include/bazzulto/keyboard.h"

// Defined in exception_vectors.S — the Assembly table we install.
extern void exception_vectors(void);

// ESR_EL1 fields — ARM ARM D13.2.36
#define ESR_EC(esr)    (((esr) >> 26) & 0x3F)  // Exception Class [31:26]
#define ESR_ISS(esr)   ((esr) & 0x1FFFFFF)      // Instruction Specific Syndrome [24:0]
#define ESR_WNR(esr)   (((esr) >> 6) & 1)       // Write-not-Read for data aborts [6]
#define ESR_DFSC(esr)  ((esr) & 0x3F)           // Data Fault Status Code [5:0]

// Exception Classes — ARM ARM D13.2.36, Table D13-3
#define EC_UNKNOWN          0x00
#define EC_WFX              0x01  // WFI/WFE trapped
#define EC_ILLEGAL_STATE    0x0E  // Illegal execution state
#define EC_SVC_AARCH64      0x15
#define EC_MSR_MRS          0x18  // MSR/MRS/System instruction trap
#define EC_INSN_ABORT_EL0   0x20
#define EC_INSN_ABORT_EL1   0x21
#define EC_PC_ALIGNMENT     0x22
#define EC_DATA_ABORT_EL0   0x24
#define EC_DATA_ABORT_EL1   0x25
#define EC_SP_ALIGNMENT     0x26
#define EC_SERROR           0x2F  // SError interrupt
#define EC_BRK              0x3C

// Return a short name for an Exception Class value.
// ARM ARM D13.2.36, Table D13-3.
static const char *ec_name(uint32_t ec) {
    switch (ec) {
        case EC_UNKNOWN:       return "Unknown";
        case EC_WFX:           return "WFI/WFE trapped";
        case EC_ILLEGAL_STATE: return "Illegal execution state";
        case EC_SVC_AARCH64:   return "SVC (AArch64)";
        case EC_MSR_MRS:       return "MSR/MRS/System trap";
        case EC_INSN_ABORT_EL0:return "Instruction abort (EL0)";
        case EC_INSN_ABORT_EL1:return "Instruction abort (EL1)";
        case EC_PC_ALIGNMENT:  return "PC alignment fault";
        case EC_DATA_ABORT_EL0:return "Data abort (EL0)";
        case EC_DATA_ABORT_EL1:return "Data abort (EL1)";
        case EC_SP_ALIGNMENT:  return "SP alignment fault";
        case EC_SERROR:        return "SError interrupt";
        case EC_BRK:           return "BRK instruction";
        default:               return "EC unknown";
    }
}

// Decode DFSC/IFSC fault type — ARM ARM D13.2.36, Table D13-5
static const char *decode_fault(uint32_t fsc) {
    switch (fsc & 0x3C) {  // bits [5:2] select fault type
        case 0x00: return "addr-size";
        case 0x04: return "translation";
        case 0x08: return "access-flag";
        case 0x0C: return "permission";
        default:   break;
    }
    if (fsc == 0x10) return "sync-external";
    if (fsc == 0x21) return "alignment";
    return "unknown";
}

// Linker-exported section boundaries — defined in kernel/arch/arm64/linker.ld.
// Used to classify ELR_EL1 without a runtime symbol table.
extern char _text_start[], _text_end[], _kernel_end[];

// Return a short description of where a virtual address lies.
// Kernel ranges come from the linker script. User range is the TTBR0 half
// (VAs below 2^48 = 0x0001000000000000) as defined by T0SZ=16 in TCR_EL1.
// ARM ARM DDI 0487 D17.2.131: VAs [63:48] = 0 are TTBR0; = 1 are TTBR1.
static const char *elr_region(uint64_t elr) {
    if (elr >= (uint64_t)_text_start && elr < (uint64_t)_text_end)
        return "kernel .text";
    if (elr >= (uint64_t)_text_end && elr < (uint64_t)_kernel_end)
        return "kernel .data/.bss";
    if (elr < 0x0001000000000000ULL)
        return "user space";
    return "unknown kernel VA";
}

static void print_exception_info(struct exception_frame *frame) {
    char line_buf[128];

    // Print register values to both framebuffer and UART.
    // ELR region identifies which part of the address space faulted without
    // requiring an embedded symbol table.
    ksnprintf(line_buf, sizeof(line_buf),
              "  ELR=0x%lx [%s]  SP=0x%lx",
              frame->elr, elr_region(frame->elr), frame->sp);
    console_println(line_buf);
    uart_puts(line_buf); uart_puts("\n");

    ksnprintf(line_buf, sizeof(line_buf),
              "  ESR=0x%lx  FAR=0x%lx", frame->esr, frame->far);
    console_println(line_buf);
    uart_puts(line_buf); uart_puts("\n");

    // Decode ESR fields.
    uint32_t ec   = ESR_EC(frame->esr);
    uint32_t dfsc = ESR_DFSC(frame->esr);

    if (ec == EC_DATA_ABORT_EL1 || ec == EC_DATA_ABORT_EL0) {
        ksnprintf(line_buf, sizeof(line_buf), "  EC=0x%02x [%s] %s %s L%u",
                  ec, ec_name(ec),
                  ESR_WNR(frame->esr) ? "WRITE" : "READ",
                  decode_fault(dfsc),
                  dfsc & 0x3u);
    } else if (ec == EC_INSN_ABORT_EL1 || ec == EC_INSN_ABORT_EL0) {
        ksnprintf(line_buf, sizeof(line_buf), "  EC=0x%02x [%s] IFETCH %s L%u",
                  ec, ec_name(ec), decode_fault(dfsc), dfsc & 0x3u);
    } else {
        ksnprintf(line_buf, sizeof(line_buf), "  EC=0x%02x [%s]",
                  ec, ec_name(ec));
    }
    console_println(line_buf);
    uart_puts(line_buf); uart_puts("\n");
}

void exception_handler_sync_el1(struct exception_frame *frame) {
    uint32_t ec = ESR_EC(frame->esr);
    char panic_buf[80];

    switch (ec) {
        case EC_SVC_AARCH64:
            uart_puts("WARNING: SVC from EL1 (unexpected system call in kernel)\n");
            console_println("WARNING: SVC from EL1 (unexpected system call in kernel)");
            return;
        default:
            ksnprintf(panic_buf, sizeof(panic_buf),
                      "KERNEL PANIC: %s", ec_name(ec));
            console_println(panic_buf);
            uart_puts(panic_buf); uart_puts("\n");
            break;
    }

    print_exception_info(frame);
    for (;;) __asm__("wfe");
}

// --- EL0 exception handlers (user space) ---

// Kill the current process and yield to the next one.
// The dead process remains in the circular queue but is skipped by the scheduler.
// Defined in systemcall.c — the PID of the init process that adopts orphans.
extern uint16_t init_process_pid;

static void process_kill_current(void) {
    process_t *p = scheduler_get_current();

    // Become a zombie (exit_status = -1 for abnormal termination).
    // Memory is freed when the parent calls wait(), or by init if orphaned.
    p->exit_status = -1;
    p->state       = PROCESS_STATE_ZOMBIE;

    if (init_process_pid != 0)
        scheduler_reparent_children(p->pid.index, init_process_pid);

    scheduler_wake_waiters(p->pid.index);
    scheduler_yield();
    // Never returns.
}

void exception_handler_sync_el0(struct exception_frame *frame) {
    uint32_t ec = ESR_EC(frame->esr);

    switch (ec) {
    case EC_SVC_AARCH64:
        systemcall_dispatch(frame);
        return;
    case EC_DATA_ABORT_EL0:
        uart_puts("[kernel] killed pid: data abort from EL0\n");
        print_exception_info(frame);
        process_kill_current();
        return;
    case EC_INSN_ABORT_EL0:
        uart_puts("[kernel] killed pid: instruction abort from EL0\n");
        print_exception_info(frame);
        process_kill_current();
        return;
    default:
        uart_puts("[kernel] killed pid: unhandled exception from EL0\n");
        print_exception_info(frame);
        process_kill_current();
        return;
    }
}

void exception_handler_irq_el0(struct exception_frame *frame) {
    // IRQ dispatch is identical regardless of which EL was preempted.
    // The eret at the end restores SPSR_EL1 which has M=EL0t,
    // so the CPU returns to user mode automatically.
    exception_handler_irq_el1(frame);
    // Deliver any pending signals before returning to EL0.
    // Timer preemption is a natural delivery point for async signals.
    systemcall_deliver_pending_signals(frame);
}

void exception_handler_irq_el1(struct exception_frame *frame) {
    (void)frame;

    // Read GICC_IAR to acknowledge and get the interrupt ID.
    // This must happen exactly ONCE per IRQ — IHI 0048B §4.4.4.
    uint32_t iar = GICC_IAR;
    uint32_t intid = iar & 0x3FF;

    switch (intid) {
    case IRQ_TIMER_EL1_PHYS:
        timer_handle_irq();
        scheduler_tick();
        break;
    case IRQ_UART0:
        uart_irq_handler();
        break;
    case IRQ_SPURIOUS:
        return;  // Do NOT write EOIR for spurious interrupts
    default:
        // Route to the keyboard driver if the INTID matches the virtio-input
        // device registered during keyboard_init(). The INTID is in the range
        // IRQ_VIRTIO_MMIO_BASE..IRQ_VIRTIO_MMIO_BASE+31 (INTID 48-79).
        // keyboard_get_irq_intid() returns 0 if no keyboard was found, so
        // this comparison is safe on the run-serial target.
        if (intid == keyboard_get_irq_intid())
            keyboard_irq_handler();
        break;
    }

    // Signal End of Interrupt — must write the original IAR value.
    GICC_EOIR = iar;
}

void exception_handler_unexpected(struct exception_frame *frame) {
    console_println("KERNEL PANIC: unexpected exception (group A/D/FIQ/SError)");
    print_exception_info(frame);
    for (;;) __asm__("wfe");
}

void exceptions_init(void) {
    __asm__ volatile(
        "msr vbar_el1, %0\n"
        "isb\n"
        :
        : "r"(exception_vectors)
        : "memory"
    );
    console_println("Exceptions: ok");
}
