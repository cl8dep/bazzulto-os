#include "../../../../include/bazzulto/syscall.h"
#include "../../../../include/bazzulto/uart.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/console.h"

// Extract the SVC immediate from ESR_EL1.
// ARM ARM D13.2.36: for EC=0x15 (SVC AArch64), ISS[15:0] = imm16.
#define ESR_SVC_IMM(esr) ((esr) & 0xFFFF)

// Maximum valid user-space address (48-bit VA, TTBR0 range).
#define USER_ADDR_LIMIT 0x0001000000000000ULL

static int validate_user_buffer(uint64_t addr, size_t len) {
	if (addr >= USER_ADDR_LIMIT) return 0;
	if (addr + len < addr) return 0;       // overflow
	if (addr + len > USER_ADDR_LIMIT) return 0;
	return 1;
}

// --- Syscall implementations ---

static int64_t sys_exit(int status) {
	(void)status;
	process_t *p = scheduler_get_current();
	p->state = PROCESS_STATE_DEAD;
	scheduler_yield();
	// Never reached
	return 0;
}

static int64_t sys_write(int fd, const char *buf, size_t len) {
	if (fd != 1) return -1;  // only stdout for now
	if (!validate_user_buffer((uint64_t)buf, len)) return -1;

	for (size_t i = 0; i < len; i++)
		uart_putc(buf[i]);
	return (int64_t)len;
}

static int64_t sys_read(int fd, char *buf, size_t len) {
	if (fd != 0) return -1;  // only stdin for now
	if (!validate_user_buffer((uint64_t)buf, len)) return -1;
	if (len == 0) return 0;

	// Read one character (blocking via UART IRQ + wait queue).
	buf[0] = uart_getc();
	return 1;
}

static int64_t sys_yield(void) {
	scheduler_yield();
	return 0;
}

// --- Dispatch ---

void syscall_dispatch(struct exception_frame *frame) {
	uint32_t nr = ESR_SVC_IMM(frame->esr);

	switch (nr) {
	case SYS_EXIT:
		sys_exit((int)frame->x0);
		break;
	case SYS_WRITE:
		frame->x0 = (uint64_t)sys_write((int)frame->x0,
		                                  (const char *)frame->x1,
		                                  (size_t)frame->x2);
		break;
	case SYS_READ:
		frame->x0 = (uint64_t)sys_read((int)frame->x0,
		                                 (char *)frame->x1,
		                                 (size_t)frame->x2);
		break;
	case SYS_YIELD:
		frame->x0 = (uint64_t)sys_yield();
		break;
	default:
		frame->x0 = (uint64_t)-1;  // unknown syscall
		break;
	}
}
