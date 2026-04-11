#include "../../../../include/bazzulto/systemcall.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/pid.h"
#include "../../../../include/bazzulto/virtual_file_system.h"
#include "../../../../include/bazzulto/ramfs.h"
#include "../../../../include/bazzulto/elf_loader.h"
#include "../../../../include/bazzulto/physical_memory.h"
#include "../../../../include/bazzulto/virtual_memory.h"
#include "../../../../include/bazzulto/kernel.h"
#include "../../../../include/bazzulto/timer.h"
#include <string.h>

// Signal return trampoline VA — a single `svc #SYSTEMCALL_SIGRETURN` instruction
// mapped read+execute at this address in every user process by elf_loader_build_image.
// The signal handler's LR is set to this address so returning from the handler
// automatically invokes sys_sigreturn.
#define SIGNAL_TRAMPOLINE_VA 0x1000ULL

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

// PID index of the init process (the shell). Orphaned children are reparented
// to this process so their zombies get reaped eventually.
// Set by kernel_main via systemcall_set_init_process() after launching the shell.
// Also exported to exceptions.c for process_kill_current().
uint16_t init_process_pid = 0;

void systemcall_set_init_process(uint16_t pid_index)
{
	init_process_pid = pid_index;
}

static int64_t sys_exit(int status)
{
	process_t *dying = scheduler_get_current();

	// Save exit status and become a zombie — memory is NOT freed yet.
	// The parent will read the status via sys_wait and then reap us.
	dying->exit_status = status;
	dying->state       = PROCESS_STATE_ZOMBIE;

	// Increment the parent's zombie_count so the cap in sys_spawn can enforce
	// a limit on un-reaped children. The parent decrements it in sys_wait.
	process_t *parent = scheduler_find_process(dying->parent_pid);
	if (parent && parent->zombie_count < (uint16_t)0xFFFF)
		parent->zombie_count++;

	// Reparent any children of this process to PID 1 (the shell / init).
	// This prevents their zombie entries from leaking if this process dies
	// before calling wait() on them.
	if (init_process_pid != 0)
		scheduler_reparent_children(dying->pid.index, init_process_pid);

	// Wake the parent if it is blocked in wait() for us.
	scheduler_wake_waiters(dying->pid.index);

	scheduler_yield();
	// Never reached
	return 0;
}

// Block the calling process until a child exits.
//
// raw_pid == -1  : wait for ANY child (POSIX wait() semantics).
// raw_pid >= 0   : wait for the specific child with that PID index.
//
// Returns the child's exit status on success, or -1 if no matching child
// exists or no children are present at all.
static int64_t sys_wait(int64_t raw_pid)
{
	process_t *caller = scheduler_get_current();

	if (raw_pid == -1) {
		// Wait for any child process to become a zombie.
		while (1) {
			process_t *zombie = scheduler_find_zombie_child(caller->pid.index);
			if (zombie) {
				int64_t exit_status = (int64_t)zombie->exit_status;
				scheduler_reap_process(zombie);
				if (caller->zombie_count > 0)
					caller->zombie_count--;
				return exit_status;
			}

			// No zombie children yet. If no children exist at all, give up.
			if (!scheduler_has_child(caller->pid.index))
				return -1;

			// At least one child still running — block until any child exits.
			// 0xFFFF is the sentinel meaning "waiting for any child".
			caller->waiting_for_pid = 0xFFFF;
			caller->state = PROCESS_STATE_WAITING;
			scheduler_yield();
			caller->waiting_for_pid = 0;
		}
	}

	// Specific-PID path.
	uint16_t target_pid_index = (uint16_t)raw_pid;
	process_t *caller_ref     = scheduler_get_current();

	// Find the target — it may already be a zombie.
	process_t *target = scheduler_find_process(target_pid_index);
	if (target == NULL)
		return -1;  // no such process

	// If not yet a zombie, block until it exits.
	if (target->state != PROCESS_STATE_ZOMBIE) {
		caller_ref->waiting_for_pid = target_pid_index;
		caller_ref->state = PROCESS_STATE_WAITING;
		scheduler_yield();
		// Resumed here by scheduler_wake_waiters after the child becomes zombie.
		caller_ref->waiting_for_pid = 0;
	}

	// Re-find the target after rescheduling (pointer still valid — zombies are
	// not freed until reaped here).
	target = scheduler_find_process(target_pid_index);
	if (target == NULL || target->state != PROCESS_STATE_ZOMBIE)
		return -1;

	int64_t exit_status = (int64_t)target->exit_status;
	scheduler_reap_process(target);
	if (caller_ref->zombie_count > 0)
		caller_ref->zombie_count--;

	return exit_status;
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

	// Enforce the zombie cap: if this process already has ZOMBIE_COUNT_MAX
	// un-reaped children, refuse to spawn more until the parent calls wait().
	process_t *spawner = scheduler_get_current();
	if (spawner->zombie_count >= ZOMBIE_COUNT_MAX)
		return -1;

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

	// Record the parent so the child's zombie can be reaped by sys_wait.
	p->parent_pid = scheduler_get_current()->pid.index;

	return (int64_t)p->pid.index;
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

// ---------------------------------------------------------------------------
// sys_pipe — create a kernel pipe and hand back two fds via user pointer
// ---------------------------------------------------------------------------

static int64_t sys_pipe(uint64_t user_fds_ptr)
{
	// user_fds_ptr must point to a writable int[2] in user space.
	if (!validate_user_buffer(user_fds_ptr, 2 * sizeof(int)))
		return -1;

	process_t *process = scheduler_get_current();
	int read_fd = -1, write_fd = -1;

	if (virtual_file_system_pipe(process->fds,
	                              &read_fd, &write_fd) < 0)
		return -1;

	int *out = (int *)user_fds_ptr;
	out[0] = read_fd;
	out[1] = write_fd;
	return 0;
}

// ---------------------------------------------------------------------------
// sys_dup / sys_dup2
// ---------------------------------------------------------------------------

static int64_t sys_dup(int oldfd)
{
	process_t *process = scheduler_get_current();
	return (int64_t)virtual_file_system_dup(process->fds, oldfd);
}

static int64_t sys_dup2(int oldfd, int newfd)
{
	process_t *process = scheduler_get_current();
	return (int64_t)virtual_file_system_dup2(process->fds,
	                                          oldfd, newfd);
}

// ---------------------------------------------------------------------------
// sys_mmap — anonymous page allocation for user heap / dynamic buffers
// ---------------------------------------------------------------------------

static uint64_t sys_mmap(uint64_t length)
{
	if (length == 0)
		return (uint64_t)-1;

	// Round up to page boundary.
	uint64_t n_pages = (length + PAGE_SIZE - 1) / PAGE_SIZE;

	process_t *process = scheduler_get_current();

	// Find a free slot in the mmap region table.
	int slot = -1;
	for (int i = 0; i < PROCESS_MMAP_MAX_REGIONS; i++) {
		if (process->mmap_regions[i].n_pages == 0) {
			slot = i;
			break;
		}
	}
	if (slot < 0)
		return (uint64_t)-1;  // too many active mappings

	uint64_t base_vaddr = process->mmap_next_vaddr;

	// Allocate and map pages.
	for (uint64_t i = 0; i < n_pages; i++) {
		void *phys = physical_memory_alloc();
		if (!phys)
			return (uint64_t)-1;  // partial failure — leak for now (no rollback)

		// Zero the page so user sees fresh memory.
		uint8_t *virt = (uint8_t *)PHYSICAL_TO_VIRTUAL(phys);
		memset(virt, 0, PAGE_SIZE);

		virtual_memory_map(process->page_table,
		                   base_vaddr + i * PAGE_SIZE,
		                   (uint64_t)phys,
		                   PAGE_FLAGS_USER_DATA);
	}

	// Record the region so munmap can free it.
	process->mmap_regions[slot].vaddr   = base_vaddr;
	process->mmap_regions[slot].n_pages = n_pages;

	// Advance the bump pointer for the next allocation.
	process->mmap_next_vaddr = base_vaddr + n_pages * PAGE_SIZE;

	return base_vaddr;
}

// ---------------------------------------------------------------------------
// sys_munmap — release an anonymous mapping returned by sys_mmap
// ---------------------------------------------------------------------------

static int64_t sys_munmap(uint64_t vaddr)
{
	process_t *process = scheduler_get_current();

	// Find the matching region.
	for (int i = 0; i < PROCESS_MMAP_MAX_REGIONS; i++) {
		if (process->mmap_regions[i].vaddr   == vaddr &&
		    process->mmap_regions[i].n_pages != 0) {
			virtual_memory_unmap_range(process->page_table,
			                           vaddr,
			                           process->mmap_regions[i].n_pages);
			process->mmap_regions[i].n_pages = 0;
			process->mmap_regions[i].vaddr   = 0;
			return 0;
		}
	}

	return -1;  // address not found in this process's mmap table
}

// ---------------------------------------------------------------------------
// sys_exec — replace the calling process image with a new ELF from ramfs
// ---------------------------------------------------------------------------

// sys_exec — replace current process image with a new ELF from ramfs.
// user_argv: optional NULL-terminated argv array in user space (NULL = {path}).
static int64_t sys_exec(const char *path, const char *const *user_argv,
                         struct exception_frame *frame)
{
	char safe_path[256];
	if (!validate_user_string((uint64_t)path, sizeof(safe_path)))
		return -1;

	size_t path_len = 0;
	while (path_len < sizeof(safe_path) - 1 && path[path_len]) {
		safe_path[path_len] = path[path_len];
		path_len++;
	}
	safe_path[path_len] = '\0';

	const struct ramfs_file *file = ramfs_lookup(safe_path);
	if (!file)
		return -1;

	// Parse user argv (same logic as sys_spawn).
	char arg_storage[SPAWN_MAX_ARGC][SPAWN_MAX_ARG_LEN];
	const char *kargv[SPAWN_MAX_ARGC + 1];
	int argc = 0;

	if (user_argv) {
		for (int i = 0; i < SPAWN_MAX_ARGC; i++) {
			uint64_t ptr_addr = (uint64_t)&user_argv[i];
			if (!validate_user_buffer(ptr_addr, sizeof(char *)))
				break;
			const char *str = user_argv[i];
			if (!str)
				break;
			if (!validate_user_string((uint64_t)str, SPAWN_MAX_ARG_LEN))
				break;
			copy_user_string(arg_storage[i], str, SPAWN_MAX_ARG_LEN);
			kargv[i] = arg_storage[i];
			argc++;
		}
	}
	if (argc == 0) {
		kargv[0] = safe_path;
		argc = 1;
	}
	kargv[argc] = NULL;

	process_t *process = scheduler_get_current();

	uint64_t *new_table = NULL;
	uint64_t  new_entry = 0;
	uint64_t  new_sp    = 0;

	if (elf_loader_build_image(file->data, file->size,
	                            kargv, argc,
	                            &new_table, &new_entry, &new_sp) < 0)
		return -1;

	// Replace the address space.  Free the old TTBR0 and reset mmap state.
	scheduler_free_user_address_space(process);

	// Close all fds and reset mmap regions so the new image starts fresh.
	virtual_file_system_close_all_fds(process->fds);
	virtual_file_system_init_fds(process->fds);
	for (int i = 0; i < PROCESS_MMAP_MAX_REGIONS; i++) {
		process->mmap_regions[i].vaddr   = 0;
		process->mmap_regions[i].n_pages = 0;
	}
	process->mmap_next_vaddr = MMAP_USER_BASE;

	process->page_table = new_table;

	// Install the new page table immediately.
	virtual_memory_switch_ttbr0(new_table);

	// Modify the exception frame so that when sys_exec returns and
	// systemcall_dispatch restores the frame via restore_exception_frame_el0,
	// the CPU erets to the new entry point with the new user SP.
	frame->elr  = new_entry;
	frame->sp   = new_sp;       // SP_EL0 — restored by restore_exception_frame_el0
	frame->spsr = 0;            // EL0t, all flags clear, DAIF=0
	frame->x0   = 0;            // return value of exec in the new image
	frame->x1   = 0;
	frame->x2   = 0;
	frame->x3   = 0;
	frame->x4   = 0;
	frame->x5   = 0;

	// Return 0 to tell dispatch not to overwrite frame->x0 again.
	// sys_exec never returns to the original user code; the eret goes to
	// the new entry point because we updated frame->elr above.
	return 0;
}

// ---------------------------------------------------------------------------
// sys_fork — fork the calling process
// ---------------------------------------------------------------------------

static int64_t sys_fork(struct exception_frame *frame)
{
	uint16_t child_pid = scheduler_fork_process(frame);
	if (child_pid == 0)
		return -1;
	return (int64_t)child_pid;
}

// ---------------------------------------------------------------------------
// sys_getpid / sys_getppid
// ---------------------------------------------------------------------------

static int64_t sys_getpid(void)
{
	return (int64_t)scheduler_get_current()->pid.index;
}

static int64_t sys_getppid(void)
{
	return (int64_t)scheduler_get_current()->parent_pid;
}

// ---------------------------------------------------------------------------
// sys_clock_gettime — read monotonic time from the architected timer
//
// clock_id is ignored (both CLOCK_REALTIME and CLOCK_MONOTONIC return the
// same monotonic counter since boot — no wall-clock epoch is available).
// ARM ARM D11.2: CNTPCT_EL0 counts at CNTFRQ_EL0 ticks per second.
// ---------------------------------------------------------------------------

static int64_t sys_clock_gettime(int clock_id, struct timespec *user_tp)
{
	(void)clock_id;
	if (!validate_user_buffer((uint64_t)user_tp, sizeof(struct timespec)))
		return -1;

	uint64_t freq  = timer_read_cntfrq();
	uint64_t count = timer_read_cntpct();

	user_tp->tv_sec  = (int64_t)(count / freq);
	user_tp->tv_nsec = (int64_t)((count % freq) * 1000000000ULL / freq);
	return 0;
}

// ---------------------------------------------------------------------------
// sys_nanosleep — sleep for at least *req time
//
// Implemented as a yield-loop: the process gives up the CPU on each tick
// and checks the deadline after each reschedule. This avoids wasting cycles
// in a tight spin while still honouring the requested duration within one
// scheduler tick (TIMER_TICK_MS = 10 ms).
// ---------------------------------------------------------------------------

static int64_t sys_nanosleep(const struct timespec *user_req,
                               struct timespec *user_rem)
{
	if (!validate_user_buffer((uint64_t)user_req, sizeof(struct timespec)))
		return -1;

	uint64_t freq  = timer_read_cntfrq();
	uint64_t ticks = (uint64_t)user_req->tv_sec * freq +
	                 (uint64_t)user_req->tv_nsec * freq / 1000000000ULL;
	uint64_t deadline = timer_read_cntpct() + ticks;

	while (timer_read_cntpct() < deadline)
		scheduler_yield();

	if (user_rem && validate_user_buffer((uint64_t)user_rem, sizeof(struct timespec))) {
		user_rem->tv_sec  = 0;
		user_rem->tv_nsec = 0;
	}
	return 0;
}

// ---------------------------------------------------------------------------
// sys_sigaction — register a signal handler
//
// handler_va: 0 = SIG_DFL (default action), 1 = SIG_IGN, else user function VA.
// ---------------------------------------------------------------------------

static int64_t sys_sigaction(int signum, uint64_t handler_va)
{
	if (signum < 1 || signum > 31)
		return -1;
	// SIGKILL (9) and SIGSTOP (19) cannot be caught or ignored.
	if (signum == 9 || signum == 19)
		return -1;
	scheduler_get_current()->signal_handlers[signum] = handler_va;
	return 0;
}

// ---------------------------------------------------------------------------
// sys_kill — send a signal to a process
// ---------------------------------------------------------------------------

static int64_t sys_kill(int pid, int signum)
{
	if (signum < 1 || signum > 31)
		return -1;
	process_t *target = scheduler_find_process((uint16_t)pid);
	if (!target)
		return -1;
	target->pending_signals |= ((uint64_t)1 << signum);
	// If target is blocked, make it runnable so it can handle the signal.
	if (target->state == PROCESS_STATE_BLOCKED ||
	    target->state == PROCESS_STATE_WAITING)
		target->state = PROCESS_STATE_READY;
	return 0;
}

// ---------------------------------------------------------------------------
// sys_sigreturn — restore CPU state after a signal handler returns
//
// The signal delivery path pushed a struct signal_frame onto the user stack
// before invoking the handler. This syscall pops that frame and restores
// the original exception_frame so the eret goes back to the interrupted code.
//
// struct signal_frame layout (matches signal delivery in
// systemcall_deliver_pending_signals below):
//   offset  0: struct exception_frame saved_frame  (288 bytes)
//   offset 288: uint64_t signum_saved               (8 bytes)
//   offset 296: uint64_t _padding                   (8 bytes)
//   total: 304 bytes (16-byte aligned)
// ---------------------------------------------------------------------------
#define SIGNAL_FRAME_SIZE 304

static int64_t sys_sigreturn(struct exception_frame *frame)
{
	// frame->sp is the user SP after signal delivery, pointing at the sigframe.
	if (!validate_user_buffer(frame->sp, SIGNAL_FRAME_SIZE))
		return -1;

	// The saved exception_frame is at the start of the sigframe.
	const struct exception_frame *saved =
		(const struct exception_frame *)frame->sp;

	// Restore original frame in-place.
	*frame = *saved;
	// frame->sp is now the original user SP (restored from saved_frame.sp).
	return 0;
}

// ---------------------------------------------------------------------------
// systemcall_deliver_pending_signals — deliver one pending signal before eret
//
// Called after every syscall and every IRQ that preempts EL0.
// If a signal is pending and has an installed handler, this function:
//   1. Pushes a struct signal_frame onto the user stack (saves the frame).
//   2. Redirects frame->elr to the handler.
//   3. Sets frame->x0 = signum (argument to the handler).
//   4. Sets frame->x30 = SIGNAL_TRAMPOLINE_VA (return address → svc sigreturn).
// The next eret then runs the signal handler instead of returning to the
// original code. When the handler returns it hits the trampoline and calls
// sys_sigreturn, which restores the saved frame and erets back to the original.
//
// Only one signal is delivered per return to EL0; remaining signals are
// delivered on subsequent returns.
// ---------------------------------------------------------------------------

void systemcall_deliver_pending_signals(struct exception_frame *frame)
{
	process_t *process = scheduler_get_current();
	if (!process || !process->pending_signals)
		return;

	for (int signum = 1; signum <= 31; signum++) {
		if (!(process->pending_signals & ((uint64_t)1 << signum)))
			continue;

		// Clear the pending bit before delivery so re-entry is safe.
		process->pending_signals &= ~((uint64_t)1 << signum);

		uint64_t handler = process->signal_handlers[signum];

		if (handler == 0) {
			// SIG_DFL: default action.
			// SIGCHLD (17): ignore by default.
			if (signum == 17)
				continue;
			// All other signals: terminate the process.
			// sys_exit calls scheduler_yield and never returns.
			sys_exit(-signum);
		}

		if (handler == 1) {
			// SIG_IGN: silently discard.
			continue;
		}

		// Deliver to the user handler by pushing a sigframe and redirecting eret.
		// struct signal_frame is 304 bytes (16-byte aligned).
		uint64_t new_sp = (frame->sp - SIGNAL_FRAME_SIZE) & ~(uint64_t)15;

		if (!validate_user_buffer(new_sp, SIGNAL_FRAME_SIZE)) {
			// User stack exhausted — terminate rather than silently drop signal.
			sys_exit(-signum);
		}

		// The signal_frame is a plain struct on the user stack.
		// Since TTBR0 is active while in EL1 exception handlers, user VAs are
		// accessible directly (no PAN in this kernel).
		struct {
			struct exception_frame saved_frame;
			uint64_t               signum_saved;
			uint64_t               padding;
		} *sigframe = (void *)new_sp;

		sigframe->saved_frame  = *frame;
		sigframe->signum_saved = (uint64_t)signum;
		sigframe->padding      = 0;

		// Redirect the eret to the signal handler.
		frame->sp   = new_sp;
		frame->elr  = handler;
		frame->spsr = 0;  // EL0t, all flags clear, DAIF=0
		frame->x0   = (uint64_t)signum;
		frame->x30  = SIGNAL_TRAMPOLINE_VA;  // handler returns via svc #sigreturn
		// Note: only one signal per eret; remaining signals delivered next return.
		break;
	}
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
		frame->x0 = (uint64_t)sys_wait((int64_t)frame->x0);
		break;
	case SYSTEMCALL_PIPE:
		frame->x0 = (uint64_t)sys_pipe(frame->x0);
		break;
	case SYSTEMCALL_DUP:
		frame->x0 = (uint64_t)sys_dup((int)frame->x0);
		break;
	case SYSTEMCALL_DUP2:
		frame->x0 = (uint64_t)sys_dup2((int)frame->x0, (int)frame->x1);
		break;
	case SYSTEMCALL_MMAP:
		frame->x0 = sys_mmap(frame->x0);
		break;
	case SYSTEMCALL_MUNMAP:
		frame->x0 = (uint64_t)sys_munmap(frame->x0);
		break;
	case SYSTEMCALL_FORK:
		frame->x0 = (uint64_t)sys_fork(frame);
		break;
	case SYSTEMCALL_EXEC:
		frame->x0 = (uint64_t)sys_exec((const char *)frame->x0,
		                                (const char *const *)frame->x1,
		                                frame);
		break;
	case SYSTEMCALL_GETPID:
		frame->x0 = (uint64_t)sys_getpid();
		break;
	case SYSTEMCALL_GETPPID:
		frame->x0 = (uint64_t)sys_getppid();
		break;
	case SYSTEMCALL_CLOCK_GETTIME:
		frame->x0 = (uint64_t)sys_clock_gettime((int)frame->x0,
		                                          (struct timespec *)frame->x1);
		break;
	case SYSTEMCALL_NANOSLEEP:
		frame->x0 = (uint64_t)sys_nanosleep((const struct timespec *)frame->x0,
		                                     (struct timespec *)frame->x1);
		break;
	case SYSTEMCALL_SIGACTION:
		frame->x0 = (uint64_t)sys_sigaction((int)frame->x0, frame->x1);
		break;
	case SYSTEMCALL_KILL:
		frame->x0 = (uint64_t)sys_kill((int)frame->x0, (int)frame->x1);
		break;
	case SYSTEMCALL_SIGRETURN:
		// sys_sigreturn restores the frame in-place; the return value written
		// to frame->x0 here is immediately overwritten by the restored frame.
		sys_sigreturn(frame);
		// Do NOT deliver signals after sigreturn — the restored frame already
		// represents the original interrupted context; deliver on next return.
		return;
	default:
		frame->x0 = (uint64_t)-1;  // unknown syscall
		break;
	}

	// Deliver any pending signals before returning to EL0.
	// This handles signals sent by kill() from within the same syscall path.
	systemcall_deliver_pending_signals(frame);
}
