#include "../../../../include/bazzulto/exceptions.h"
#include "../../../../include/bazzulto/console.h"
#include "../../../../include/bazzulto/gic.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/timer.h"
#include "../../../../include/bazzulto/uart.h"
#include "../../../../include/bazzulto/syscall.h"

// Defined in exception_vectors.S — the Assembly table we install.
extern void exception_vectors(void);

// ESR_EL1 fields — ARM ARM D13.2.36
#define ESR_EC(esr)    (((esr) >> 26) & 0x3F)  // Exception Class [31:26]
#define ESR_ISS(esr)   ((esr) & 0x1FFFFFF)      // Instruction Specific Syndrome [24:0]
#define ESR_WNR(esr)   (((esr) >> 6) & 1)       // Write-not-Read for data aborts [6]
#define ESR_DFSC(esr)  ((esr) & 0x3F)           // Data Fault Status Code [5:0]

// Exception Classes — ARM ARM D13.2.36, Table D13-3
#define EC_DATA_ABORT_EL1   0x25
#define EC_INSN_ABORT_EL1   0x21
#define EC_DATA_ABORT_EL0   0x24
#define EC_INSN_ABORT_EL0   0x20
#define EC_SVC_AARCH64      0x15
#define EC_BRK              0x3C

// Print a 64-bit value as 16-digit hex.
static void print_hex64(uint64_t val) {
    char buf[17];
    for (int i = 15; i >= 0; i--) {
        buf[i] = "0123456789ABCDEF"[val & 0xF];
        val >>= 4;
    }
    buf[16] = '\0';
    console_print(buf);
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

// Print a hex value to UART for serial debugging.
static void uart_print_hex64(uint64_t val) {
    char buf[17];
    for (int i = 15; i >= 0; i--) {
        buf[i] = "0123456789ABCDEF"[val & 0xF];
        val >>= 4;
    }
    buf[16] = '\0';
    uart_puts(buf);
}

static void print_exception_info(struct exception_frame *frame) {
    // Print to both framebuffer and UART for debugging.
    console_print("  ELR=0x"); print_hex64(frame->elr);
    console_print("  SP=0x");  print_hex64(frame->sp);
    console_println("");
    console_print("  ESR=0x"); print_hex64(frame->esr);
    console_print("  FAR=0x"); print_hex64(frame->far);
    console_println("");

    uart_puts("  ELR=0x"); uart_print_hex64(frame->elr);
    uart_puts(" SP=0x");   uart_print_hex64(frame->sp);
    uart_puts(" ESR=0x");  uart_print_hex64(frame->esr);
    uart_puts(" FAR=0x");  uart_print_hex64(frame->far);
    uart_puts("\n");

    // Decode ESR fields
    uint32_t ec   = ESR_EC(frame->esr);
    uint32_t dfsc = ESR_DFSC(frame->esr);

    console_print("  EC=0x");
    char ecbuf[3];
    ecbuf[0] = "0123456789ABCDEF"[(ec >> 4) & 0xF];
    ecbuf[1] = "0123456789ABCDEF"[ec & 0xF];
    ecbuf[2] = '\0';
    console_print(ecbuf);

    if (ec == EC_DATA_ABORT_EL1 || ec == EC_DATA_ABORT_EL0) {
        console_print(ESR_WNR(frame->esr) ? " WRITE " : " READ ");
        console_print(decode_fault(dfsc));
        console_print(" L");
        char lvl = '0' + (dfsc & 0x3);
        console_print((char[]){lvl, '\0'});
    } else if (ec == EC_INSN_ABORT_EL1 || ec == EC_INSN_ABORT_EL0) {
        console_print(" IFETCH ");
        console_print(decode_fault(dfsc));
        console_print(" L");
        char lvl = '0' + (dfsc & 0x3);
        console_print((char[]){lvl, '\0'});
    }
    console_println("");
}

void exception_handler_sync_el1(struct exception_frame *frame) {
    uint32_t ec = ESR_EC(frame->esr);

    switch (ec) {
        case EC_DATA_ABORT_EL1:
            console_println("KERNEL PANIC: data abort");
            break;
        case EC_INSN_ABORT_EL1:
            console_println("KERNEL PANIC: instruction abort");
            break;
        case EC_SVC_AARCH64:
            console_println("WARNING: SVC from EL1 (unexpected syscall in kernel)");
            return;
        default:
            console_println("KERNEL PANIC: unhandled synchronous exception");
            break;
    }

    print_exception_info(frame);
    for (;;) __asm__("wfe");
}

// --- EL0 exception handlers (user space) ---

// Kill the current process and yield to the next one.
// The dead process remains in the circular queue but is skipped by the scheduler.
static void process_kill_current(void) {
    process_t *p = scheduler_get_current();
    p->state = PROCESS_STATE_DEAD;
    // TODO: free user page table and physical pages
    scheduler_yield();
    // Never returns — the process is DEAD and will never be scheduled again.
}

void exception_handler_sync_el0(struct exception_frame *frame) {
    uint32_t ec = ESR_EC(frame->esr);

    switch (ec) {
    case EC_SVC_AARCH64:
        syscall_dispatch(frame);
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
