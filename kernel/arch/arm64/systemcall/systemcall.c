#include "../../../../include/bazzulto/systemcall.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/virtual_file_system.h"
#include "../../../../include/bazzulto/ramfs.h"
#include "../../../../include/bazzulto/elf_loader.h"

// Extract the SVC immediate from ESR_EL1.
// ARM ARM D13.2.36: for EC=0x15 (SVC AArch64), ISS[15:0] = imm16.
#define ESR_SVC_IMM(esr) ((esr) & 0xFFFF)

// Maximum valid user-space address (48-bit VA, TTBR0 range).
#define USER_ADDR_LIMIT 0x0001000000000000ULL

static int validate_user_buffer(uint64_t addr, size_t len)
{
	if (addr >= USER_ADDR_LIMIT) return 0;
	if (addr + len < addr) return 0;       // overflow
	if (addr + len > USER_ADDR_LIMIT) return 0;
	return 1;
}

// Validate a null-terminated user string (up to max_len bytes).
// Returns 1 if valid, 0 if the string extends beyond user address space.
static int validate_user_string(uint64_t addr, size_t max_len)
{
	if (addr >= USER_ADDR_LIMIT) return 0;
	const char *s = (const char *)addr;
	for (size_t i = 0; i < max_len; i++) {
		if (addr + i >= USER_ADDR_LIMIT) return 0;
		if (s[i] == '\0') return 1;
	}
	return 0; // no null terminator within max_len
}

// --- Syscall implementations ---

static int64_t sys_exit(int status)
{
	(void)status;
	process_t *dying = scheduler_get_current();
	dying->state = PROCESS_STATE_DEAD;

	// Wake any process that is blocked in wait() for this PID.
	// We must scan the run queue because we have no parent pointer —
	// any process could be waiting for us.
	scheduler_wake_waiters(dying->pid);

	scheduler_yield();
	// Never reached
	return 0;
}

// Block the calling process until the process with `target_pid` exits.
// If the target is already DEAD by the time we check, return immediately.
static int64_t sys_wait(uint32_t target_pid)
{
	process_t *current = scheduler_get_current();

	// Fast path: the target process already exited before we even called wait().
	if (scheduler_find_process(target_pid) == NULL ||
	    scheduler_find_process(target_pid)->state == PROCESS_STATE_DEAD) {
		return 0;
	}

	// Record which PID we are waiting for and block.
	current->waiting_for_pid = target_pid;
	current->state = PROCESS_STATE_WAITING;
	scheduler_yield();

	// Execution resumes here after scheduler_wake_waiters() sets us READY.
	current->waiting_for_pid = 0;
	return 0;
}

static int64_t sys_write(int fd, const char *buf, size_t len)
{
	if (!validate_user_buffer((uint64_t)buf, len)) return -1;
	process_t *p = scheduler_get_current();
	return virtual_file_system_write(p->fds, fd, buf, len);
}

static int64_t sys_read(int fd, char *buf, size_t len)
{
	if (!validate_user_buffer((uint64_t)buf, len)) return -1;
	if (len == 0) return 0;
	process_t *p = scheduler_get_current();
	return virtual_file_system_read(p->fds, fd, buf, len);
}

static int64_t sys_yield(void)
{
	scheduler_yield();
	return 0;
}

static int64_t sys_open(const char *path)
{
	if (!validate_user_string((uint64_t)path, 256)) return -1;
	process_t *p = scheduler_get_current();
	return virtual_file_system_open(p->fds, path);
}

static int64_t sys_close(int fd)
{
	process_t *p = scheduler_get_current();
	return virtual_file_system_close(p->fds, fd);
}

static int64_t sys_seek(int fd, int64_t offset, int whence)
{
	process_t *p = scheduler_get_current();
	return virtual_file_system_seek(p->fds, fd, offset, whence);
}

// Maximum number of arguments for SYS_SPAWN.
#define SPAWN_MAX_ARGC  32
// Maximum length of a single argument string.
#define SPAWN_MAX_ARG_LEN 256

// Simple kernel-side string copy. Returns bytes copied (including '\0').
static size_t copy_user_string(char *dst, const char *src, size_t max)
{
	size_t i = 0;
	while (i < max - 1 && src[i]) {
		dst[i] = src[i];
		i++;
	}
	dst[i] = '\0';
	return i + 1;
}

static int64_t sys_spawn(const char *path, const char *const *user_argv)
{
	if (!validate_user_string((uint64_t)path, SPAWN_MAX_ARG_LEN)) return -1;

	const struct ramfs_file *file = ramfs_lookup(path);
	if (!file) return -1;

	// Copy argv from the caller's user space into kernel memory.
	// We read the argv array and strings while the caller's TTBR0 is active.
	char arg_storage[SPAWN_MAX_ARGC][SPAWN_MAX_ARG_LEN];
	const char *kargv[SPAWN_MAX_ARGC + 1];
	int argc = 0;

	if (user_argv) {
		for (int i = 0; i < SPAWN_MAX_ARGC; i++) {
			// Read the pointer at user_argv[i].
			uint64_t ptr_addr = (uint64_t)&user_argv[i];
			if (!validate_user_buffer(ptr_addr, sizeof(char *)))
				break;

			const char *str = user_argv[i];
			if (str == NULL)
				break;

			if (!validate_user_string((uint64_t)str, SPAWN_MAX_ARG_LEN))
				break;

			copy_user_string(arg_storage[i], str, SPAWN_MAX_ARG_LEN);
			kargv[i] = arg_storage[i];
			argc++;
		}
	}
	kargv[argc] = NULL;

	process_t *p = elf_loader_load(file->data, file->size,
	                                argc > 0 ? kargv : NULL, argc);
	if (!p) return -1;

	return (int64_t)p->pid;
}

static int64_t sys_list(int index, char *name_buf, size_t buf_len)
{
	if (!validate_user_buffer((uint64_t)name_buf, buf_len)) return -1;
	if (buf_len == 0) return -1;

	const struct ramfs_file *file = ramfs_file_at(index);
	if (!file) return -1;

	// Copy file name into user buffer (truncate if needed).
	const char *name = file->name;
	size_t i = 0;
	while (i < buf_len - 1 && name[i]) {
		name_buf[i] = name[i];
		i++;
	}
	name_buf[i] = '\0';

	return (int64_t)file->size;
}

// --- Dispatch ---

void systemcall_dispatch(struct exception_frame *frame)
{
	uint32_t nr = ESR_SVC_IMM(frame->esr);

	switch (nr) {
	case SYSTEMCALL_EXIT:
		sys_exit((int)frame->x0);
		break;
	case SYSTEMCALL_WRITE:
		frame->x0 = (uint64_t)sys_write((int)frame->x0,
		                                  (const char *)frame->x1,
		                                  (size_t)frame->x2);
		break;
	case SYSTEMCALL_READ:
		frame->x0 = (uint64_t)sys_read((int)frame->x0,
		                                 (char *)frame->x1,
		                                 (size_t)frame->x2);
		break;
	case SYSTEMCALL_YIELD:
		frame->x0 = (uint64_t)sys_yield();
		break;
	case SYSTEMCALL_OPEN:
		frame->x0 = (uint64_t)sys_open((const char *)frame->x0);
		break;
	case SYSTEMCALL_CLOSE:
		frame->x0 = (uint64_t)sys_close((int)frame->x0);
		break;
	case SYSTEMCALL_SEEK:
		frame->x0 = (uint64_t)sys_seek((int)frame->x0,
		                                 (int64_t)frame->x1,
		                                 (int)frame->x2);
		break;
	case SYSTEMCALL_SPAWN:
		frame->x0 = (uint64_t)sys_spawn((const char *)frame->x0,
		                                  (const char *const *)frame->x1);
		break;
	case SYSTEMCALL_LIST:
		frame->x0 = (uint64_t)sys_list((int)frame->x0,
		                                 (char *)frame->x1,
		                                 (size_t)frame->x2);
		break;
	case SYSTEMCALL_WAIT:
		frame->x0 = (uint64_t)sys_wait((uint32_t)frame->x0);
		break;
	default:
		frame->x0 = (uint64_t)-1;  // unknown syscall
		break;
	}
}
