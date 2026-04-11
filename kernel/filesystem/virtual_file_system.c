#include "../../include/bazzulto/virtual_file_system.h"
#include "../../include/bazzulto/ramfs.h"
#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/fs_ram.h"
#include "../../include/bazzulto/hal/hal_uart.h"
#include "../../include/bazzulto/input.h"
#include "../../include/bazzulto/tty.h"
#include "../../include/bazzulto/console.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/scheduler.h"
#include "errno.h"
#include <string.h>

// Forward declaration for FAT32 disk file close.
extern void fs_disk_close(void *disk_file);
extern int64_t fs_disk_read(void *file, char *buf, uint64_t offset, uint64_t len);
extern int64_t fs_disk_write(void *file, const char *buf, uint64_t offset, uint64_t len);
extern int64_t fs_disk_fstat_size(const file_descriptor_t *fd);

// Forward declarations for pipe helpers defined later in this file.
static int64_t pipe_read(file_descriptor_t *fds, pipe_buffer_t *buf,
                          char *out, size_t len);
static int64_t pipe_write(file_descriptor_t *fds, pipe_buffer_t *buf,
                           const char *in, size_t len);

void virtual_file_system_init_fds(file_descriptor_t *fds)
{
	// Clear all slots.
	for (int i = 0; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		fds[i].type = FD_TYPE_NONE;
		fds[i].file = NULL;
		fds[i].offset = 0;
	}

	// Reserve fd 0 (stdin), 1 (stdout), 2 (stderr) as console I/O.
	fds[0].type = FD_TYPE_CONSOLE;
	fds[1].type = FD_TYPE_CONSOLE;
	fds[2].type = FD_TYPE_CONSOLE;
}

// Find the lowest available fd slot (starting from 3).
static int alloc_fd(file_descriptor_t *fds)
{
	for (int i = 3; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_NONE)
			return i;
	}
	return -1; // No free slots
}

int virtual_file_system_open(file_descriptor_t *fds, const char *path)
{
	int fd = alloc_fd(fds);
	if (fd < 0)
		return -EMFILE;

	int result = vfs_scheme_open(path, &fds[fd]);
	if (result < 0) {
		fds[fd].type = FD_TYPE_NONE;
		return result;
	}
	return fd;
}

int virtual_file_system_creat(file_descriptor_t *fds, const char *path)
{
	int fd = alloc_fd(fds);
	if (fd < 0)
		return -EMFILE;

	int result = vfs_scheme_creat(path, &fds[fd]);
	if (result < 0) {
		fds[fd].type = FD_TYPE_NONE;
		return result;
	}
	return fd;
}

int virtual_file_system_unlink(const char *path)
{
	return vfs_scheme_unlink(path);
}

int virtual_file_system_fstat(file_descriptor_t *fds, int fd,
                               struct vfs_stat *stat_out)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -EBADF;
	if (fds[fd].type == FD_TYPE_NONE)
		return -EBADF;

	int64_t size = vfs_scheme_fstat_size(&fds[fd]);
	if (size < 0)
		return (int)size;
	stat_out->size = (uint64_t)size;
	stat_out->type = 0;  // regular file
	return 0;
}

int64_t virtual_file_system_read(file_descriptor_t *fds, int fd, char *buf, size_t len)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -EBADF;
	if (fds[fd].type == FD_TYPE_NONE)
		return -EBADF;

	if (fds[fd].type == FD_TYPE_CONSOLE) {
		// Only stdin (fd 0) supports reading via the TTY layer.
		if (fd != 0)
			return -EBADF;
		if (len == 0)
			return 0;
		int64_t result = tty_read(buf, len);
		if (result < 0)
			return 0;  // interrupted by signal — return 0 so syscall unwinds
		return result;
	}

	if (fds[fd].type == FD_TYPE_PIPE_READ)
		return pipe_read(fds, fds[fd].pipe, buf, len);

	if (fds[fd].type == FD_TYPE_PIPE_WRITE)
		return -EBADF;  // cannot read from write end

	if (fds[fd].type == FD_TYPE_DEV_NULL)
		return 0;  // always EOF

	if (fds[fd].type == FD_TYPE_DEV_ZERO) {
		for (size_t i = 0; i < len; i++)
			buf[i] = 0;
		return (int64_t)len;
	}

	if (fds[fd].type == FD_TYPE_DEV_RANDOM) {
		// CNTPCT_EL0 xorshift64 as a simple pseudorandom source.
		uint64_t state;
		__asm__ volatile("mrs %0, cntpct_el0" : "=r"(state));
		state ^= (uint64_t)fds[fd].offset;  // mix in offset for variety
		for (size_t i = 0; i < len; i++) {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			buf[i] = (char)(state & 0xFF);
		}
		fds[fd].offset += len;
		return (int64_t)len;
	}

	if (fds[fd].type == FD_TYPE_RAM_FILE) {
		ram_inode_t *inode = (ram_inode_t *)fds[fd].ram_file;
		if (!inode) return -EBADF;
		size_t offset = fds[fd].offset;
		if (offset >= (size_t)inode->size) return 0;
		size_t available = (size_t)inode->size - offset;
		size_t to_read = (len < available) ? len : available;
		for (size_t i = 0; i < to_read; i++)
			buf[i] = (char)inode->data[offset + i];
		fds[fd].offset += to_read;
		return (int64_t)to_read;
	}

	if (fds[fd].type == FD_TYPE_PROC) {
		proc_snapshot_t *snap = fds[fd].proc;
		if (!snap) return -EBADF;
		size_t offset = fds[fd].offset;
		if (offset >= snap->size) return 0;
		size_t available = snap->size - offset;
		size_t to_read = (len < available) ? len : available;
		for (size_t i = 0; i < to_read; i++)
			buf[i] = snap->buf[offset + i];
		fds[fd].offset += to_read;
		return (int64_t)to_read;
	}

	if (fds[fd].type == FD_TYPE_DISK_FILE) {
		void *disk_file = fds[fd].disk_file;
		if (!disk_file) return -EBADF;
		size_t offset = fds[fd].offset;
		int64_t result = fs_disk_read(disk_file, buf, (uint64_t)offset, (uint64_t)len);
		if (result > 0)
			fds[fd].offset += (size_t)result;
		return result;
	}

	// FD_TYPE_FILE: read from ramfs file.
	const struct ramfs_file *file = fds[fd].file;
	size_t offset = fds[fd].offset;

	if (offset >= file->size)
		return 0; // EOF

	size_t available = file->size - offset;
	size_t to_read = (len < available) ? len : available;

	// Copy from kernel memory to user buffer.
	// The caller (syscall layer) has already validated the user buffer address.
	const uint8_t *src = file->data + offset;
	for (size_t i = 0; i < to_read; i++)
		buf[i] = (char)src[i];

	fds[fd].offset += to_read;
	return (int64_t)to_read;
}

int64_t virtual_file_system_write(file_descriptor_t *fds, int fd, const char *buf, size_t len)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -EBADF;
	if (fds[fd].type == FD_TYPE_NONE)
		return -EBADF;

	if (fds[fd].type == FD_TYPE_CONSOLE) {
		// Only stdout (fd 1) and stderr (fd 2) support writing.
		if (fd != 1 && fd != 2)
			return -EBADF;
		// Write to both the framebuffer (QEMU window) and UART serial
		// (uart.log / run-serial fallback) so output is visible in both contexts.
		for (size_t i = 0; i < len; i++) {
			console_putc(buf[i]);
			hal_uart_putc(buf[i]);
		}
		return (int64_t)len;
	}

	if (fds[fd].type == FD_TYPE_PIPE_WRITE)
		return pipe_write(fds, fds[fd].pipe, buf, len);

	if (fds[fd].type == FD_TYPE_PIPE_READ)
		return -EBADF;  // cannot write to read end

	if (fds[fd].type == FD_TYPE_DEV_NULL)
		return (int64_t)len;  // discard

	if (fds[fd].type == FD_TYPE_DEV_ZERO ||
	    fds[fd].type == FD_TYPE_DEV_RANDOM)
		return -EACCES;  // not writable

	if (fds[fd].type == FD_TYPE_RAM_FILE) {
		ram_inode_t *inode = (ram_inode_t *)fds[fd].ram_file;
		if (!inode) return -EBADF;
		int64_t written = fs_ram_write(inode, (const uint8_t *)buf,
		                               (uint64_t)fds[fd].offset, (uint64_t)len);
		if (written > 0)
			fds[fd].offset += (size_t)written;
		return written;
	}

	if (fds[fd].type == FD_TYPE_DISK_FILE) {
		void *disk_file = fds[fd].disk_file;
		if (!disk_file) return -EBADF;
		int64_t result = fs_disk_write(disk_file, buf, (uint64_t)fds[fd].offset, (uint64_t)len);
		if (result > 0)
			fds[fd].offset += (size_t)result;
		return result;
	}

	// FD_TYPE_FILE (ramfs) and FD_TYPE_PROC are read-only.
	return -EACCES;
}

int virtual_file_system_close(file_descriptor_t *fds, int fd)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -EBADF;
	if (fds[fd].type == FD_TYPE_NONE)
		return -EBADF;

	// Do not allow closing stdin/stdout/stderr.
	if (fd < 3)
		return -EBADF;

	// Decrement pipe ref counts and free buffer when total reaches zero.
	if (fds[fd].type == FD_TYPE_PIPE_READ ||
	    fds[fd].type == FD_TYPE_PIPE_WRITE) {
		pipe_buffer_t *pipe = fds[fd].pipe;
		if (pipe) {
			if (fds[fd].type == FD_TYPE_PIPE_READ)
				pipe->read_ref_count--;
			pipe->ref_count--;
			if (pipe->ref_count <= 0)
				kfree(pipe);
		}
	}

	// Free proc snapshot buffer.
	if (fds[fd].type == FD_TYPE_PROC && fds[fd].proc)
		kfree(fds[fd].proc);

	// Free FAT32 disk file state.
	if (fds[fd].type == FD_TYPE_DISK_FILE && fds[fd].disk_file)
		fs_disk_close(fds[fd].disk_file);

	fds[fd].type = FD_TYPE_NONE;
	fds[fd].file = NULL;
	fds[fd].offset = 0;
	return 0;
}

int64_t virtual_file_system_seek(file_descriptor_t *fds, int fd, int64_t offset, int whence)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -EBADF;

	fd_type_t type = fds[fd].type;
	if (type != FD_TYPE_FILE && type != FD_TYPE_RAM_FILE &&
	    type != FD_TYPE_PROC && type != FD_TYPE_DISK_FILE)
		return -ESPIPE;

	int64_t file_size;
	if (type == FD_TYPE_FILE)
		file_size = (int64_t)fds[fd].file->size;
	else if (type == FD_TYPE_RAM_FILE)
		file_size = (int64_t)((ram_inode_t *)fds[fd].ram_file)->size;
	else if (type == FD_TYPE_PROC)
		file_size = (int64_t)fds[fd].proc->size;
	else
			file_size = (int64_t)fds[fd].disk_file ? fs_disk_fstat_size(&fds[fd]) : -EBADF;

	int64_t new_offset;
	switch (whence) {
	case VIRTUAL_FILE_SYSTEM_SEEK_SET:
		new_offset = offset;
		break;
	case VIRTUAL_FILE_SYSTEM_SEEK_CUR:
		new_offset = (int64_t)fds[fd].offset + offset;
		break;
	case VIRTUAL_FILE_SYSTEM_SEEK_END:
		new_offset = file_size + offset;
		break;
	default:
		return -EINVAL;
	}

	if (new_offset < 0)
		return -EINVAL;

	fds[fd].offset = (size_t)new_offset;
	return new_offset;
}

// ---------------------------------------------------------------------------
// close_all — release all FDs on process exit
// ---------------------------------------------------------------------------

void virtual_file_system_close_all_fds(file_descriptor_t *fds)
{
	for (int i = 0; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_PIPE_READ ||
		    fds[i].type == FD_TYPE_PIPE_WRITE) {
			pipe_buffer_t *pipe = fds[i].pipe;
			if (pipe) {
				if (fds[i].type == FD_TYPE_PIPE_READ)
					pipe->read_ref_count--;
				pipe->ref_count--;
				if (pipe->ref_count <= 0)
					kfree(pipe);
			}
		}
		if (fds[i].type == FD_TYPE_PROC && fds[i].proc)
			kfree(fds[i].proc);
		fds[i].type = FD_TYPE_NONE;
		fds[i].file = NULL;
		fds[i].offset = 0;
	}
}

// ---------------------------------------------------------------------------
// pipe — create a kernel ring buffer shared between two FDs
// ---------------------------------------------------------------------------

int virtual_file_system_pipe(file_descriptor_t *fds,
                              int *read_fd_out, int *write_fd_out)
{
	// Find two free slots.
	int read_fd  = -1;
	int write_fd = -1;
	for (int i = 3; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_NONE) {
			if (read_fd < 0)  { read_fd  = i; continue; }
			if (write_fd < 0) { write_fd = i; break; }
		}
	}
	if (read_fd < 0 || write_fd < 0)
		return -EMFILE;

	pipe_buffer_t *buf = (pipe_buffer_t *)kmalloc(sizeof(pipe_buffer_t));
	if (!buf)
		return -ENOMEM;

	memset(buf, 0, sizeof(pipe_buffer_t));
	buf->ref_count      = 2;  // one for each end
	buf->read_ref_count = 1;  // one read end initially

	fds[read_fd].type  = FD_TYPE_PIPE_READ;
	fds[read_fd].pipe  = buf;
	fds[read_fd].offset = 0;

	fds[write_fd].type  = FD_TYPE_PIPE_WRITE;
	fds[write_fd].pipe  = buf;
	fds[write_fd].offset = 0;

	*read_fd_out  = read_fd;
	*write_fd_out = write_fd;
	return 0;
}

// ---------------------------------------------------------------------------
// dup / dup2
// ---------------------------------------------------------------------------

// Check whether any write end of this pipe buffer is still open anywhere.
// Uses ref counts: ref_count tracks ALL ends, read_ref_count tracks read ends,
// so the difference is the number of write ends open across all processes.
static int pipe_write_end_open(const pipe_buffer_t *buf)
{
	return (buf->ref_count - buf->read_ref_count) > 0;
}

int virtual_file_system_dup(file_descriptor_t *fds, int oldfd)
{
	if (oldfd < 0 || oldfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -EBADF;
	if (fds[oldfd].type == FD_TYPE_NONE) return -EBADF;

	int newfd = -1;
	for (int i = 3; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_NONE) { newfd = i; break; }
	}
	if (newfd < 0) return -EMFILE;

	fds[newfd] = fds[oldfd];
	if (fds[newfd].type == FD_TYPE_PIPE_READ ||
	    fds[newfd].type == FD_TYPE_PIPE_WRITE) {
		fds[newfd].pipe->ref_count++;
		if (fds[newfd].type == FD_TYPE_PIPE_READ)
			fds[newfd].pipe->read_ref_count++;
	}

	return newfd;
}

int virtual_file_system_dup2(file_descriptor_t *fds, int oldfd, int newfd)
{
	if (oldfd < 0 || oldfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -EBADF;
	if (newfd < 0 || newfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -EBADF;
	if (fds[oldfd].type == FD_TYPE_NONE) return -EBADF;
	if (oldfd == newfd) return newfd;

	// Close newfd first if it's open.
	if (fds[newfd].type != FD_TYPE_NONE && newfd >= 3)
		virtual_file_system_close(fds, newfd);

	fds[newfd] = fds[oldfd];
	if (fds[newfd].type == FD_TYPE_PIPE_READ ||
	    fds[newfd].type == FD_TYPE_PIPE_WRITE) {
		fds[newfd].pipe->ref_count++;
		if (fds[newfd].type == FD_TYPE_PIPE_READ)
			fds[newfd].pipe->read_ref_count++;
	}

	return newfd;
}

// ---------------------------------------------------------------------------
// Extend read/write to handle pipe FDs
// (inject into the existing read/write functions via the FD_TYPE check)
// ---------------------------------------------------------------------------

// These helpers are called by virtual_file_system_read/write when the FD is
// a pipe. They are defined here rather than duplicating code in the main
// read/write bodies.

// Read up to len bytes from a pipe. Blocks (yield-spin) if empty.
// Returns 0 when the write end is closed and no data remains.
static int64_t pipe_read(file_descriptor_t *fds __attribute__((unused)),
                          pipe_buffer_t *buf, char *out, size_t len)
{
	if (len == 0) return 0;

	// Enable IRQs during the yield-spin loop. We are inside a syscall (SVC)
	// which enters EL1 with PSTATE.I=1 (IRQs masked). Without unmasking,
	// the timer tick never fires, so no other process can be scheduled, and
	// the pipe writer (in another process) never runs — complete deadlock.
	__asm__ volatile("msr daifclr, #2");

	size_t total = 0;
	while (total < len) {
		if (buf->count == 0) {
			// No data. If write end is closed, signal EOF.
			if (!pipe_write_end_open(buf))
				break;
			// Check for pending signals so Ctrl+C can interrupt a blocking read.
			process_t *proc = scheduler_get_current();
			if (proc && proc->pending_signals) {
				__asm__ volatile("msr daifset, #2");
				return total > 0 ? (int64_t)total : 0;
			}
			// Yield and retry — the writer will eventually produce data.
			scheduler_yield();
			continue;
		}
		out[total] = (char)buf->data[buf->read_pos];
		buf->read_pos = (buf->read_pos + 1) % PIPE_BUFFER_SIZE;
		buf->count--;
		total++;
	}

	__asm__ volatile("msr daifset, #2");
	return (int64_t)total;
}

// Write up to len bytes to a pipe. Blocks (yield-spin) if the buffer is full.
// Returns -1 (broken pipe) if all read ends are closed.
static int64_t pipe_write(file_descriptor_t *fds __attribute__((unused)),
                           pipe_buffer_t *buf,
                           const char *in, size_t len)
{
	if (len == 0) return 0;

	__asm__ volatile("msr daifclr, #2");

	size_t total = 0;
	while (total < len) {
		// Broken pipe: all readers have closed — stop writing.
		if (buf->read_ref_count <= 0) {
			__asm__ volatile("msr daifset, #2");
			return total > 0 ? (int64_t)total : -1;
		}
		if (buf->count == PIPE_BUFFER_SIZE) {
			// Buffer full — yield and retry.
			scheduler_yield();
			continue;
		}
		uint32_t write_pos = (buf->read_pos + buf->count) % PIPE_BUFFER_SIZE;
		buf->data[write_pos] = (uint8_t)in[total];
		buf->count++;
		total++;
	}

	__asm__ volatile("msr daifset, #2");
	return (int64_t)total;
}
