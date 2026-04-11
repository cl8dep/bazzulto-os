#include "../../include/bazzulto/virtual_file_system.h"
#include "../../include/bazzulto/ramfs.h"
#include "../../include/bazzulto/uart.h"
#include "../../include/bazzulto/input.h"
#include "../../include/bazzulto/console.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/scheduler.h"
#include <string.h>

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
	const struct ramfs_file *file = ramfs_lookup(path);
	if (!file)
		return -1;

	int fd = alloc_fd(fds);
	if (fd < 0)
		return -1;

	fds[fd].type = FD_TYPE_FILE;
	fds[fd].file = file;
	fds[fd].offset = 0;
	return fd;
}

int64_t virtual_file_system_read(file_descriptor_t *fds, int fd, char *buf, size_t len)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -1;
	if (fds[fd].type == FD_TYPE_NONE)
		return -1;

	if (fds[fd].type == FD_TYPE_CONSOLE) {
		// Only stdin (fd 0) supports reading.
		if (fd != 0)
			return -1;
		if (len == 0)
			return 0;
		buf[0] = input_getchar();
		return 1;
	}

	if (fds[fd].type == FD_TYPE_PIPE_READ)
		return pipe_read(fds, fds[fd].pipe, buf, len);

	if (fds[fd].type == FD_TYPE_PIPE_WRITE)
		return -1;  // cannot read from write end

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
		return -1;
	if (fds[fd].type == FD_TYPE_NONE)
		return -1;

	if (fds[fd].type == FD_TYPE_CONSOLE) {
		// Only stdout (fd 1) and stderr (fd 2) support writing.
		if (fd != 1 && fd != 2)
			return -1;
		// Write to both the framebuffer (QEMU window) and UART serial
		// (uart.log / run-serial fallback) so output is visible in both contexts.
		for (size_t i = 0; i < len; i++) {
			console_putc(buf[i]);
			uart_putc(buf[i]);
		}
		return (int64_t)len;
	}

	if (fds[fd].type == FD_TYPE_PIPE_WRITE)
		return pipe_write(fds, fds[fd].pipe, buf, len);

	if (fds[fd].type == FD_TYPE_PIPE_READ)
		return -1;  // cannot write to read end

	// ramfs files are read-only.
	return -1;
}

int virtual_file_system_close(file_descriptor_t *fds, int fd)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -1;
	if (fds[fd].type == FD_TYPE_NONE)
		return -1;

	// Do not allow closing stdin/stdout/stderr.
	if (fd < 3)
		return -1;

	// Decrement pipe ref count and free buffer when it reaches zero.
	if (fds[fd].type == FD_TYPE_PIPE_READ ||
	    fds[fd].type == FD_TYPE_PIPE_WRITE) {
		pipe_buffer_t *pipe = fds[fd].pipe;
		if (pipe) {
			pipe->ref_count--;
			if (pipe->ref_count <= 0)
				kfree(pipe);
		}
	}

	fds[fd].type = FD_TYPE_NONE;
	fds[fd].file = NULL;
	fds[fd].offset = 0;
	return 0;
}

int64_t virtual_file_system_seek(file_descriptor_t *fds, int fd, int64_t offset, int whence)
{
	if (fd < 0 || fd >= VIRTUAL_FILE_SYSTEM_MAX_FDS)
		return -1;
	if (fds[fd].type != FD_TYPE_FILE)
		return -1;

	const struct ramfs_file *file = fds[fd].file;
	int64_t new_offset;

	switch (whence) {
	case VIRTUAL_FILE_SYSTEM_SEEK_SET:
		new_offset = offset;
		break;
	case VIRTUAL_FILE_SYSTEM_SEEK_CUR:
		new_offset = (int64_t)fds[fd].offset + offset;
		break;
	case VIRTUAL_FILE_SYSTEM_SEEK_END:
		new_offset = (int64_t)file->size + offset;
		break;
	default:
		return -1;
	}

	if (new_offset < 0)
		return -1;
	if ((uint64_t)new_offset > file->size)
		new_offset = (int64_t)file->size;

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
				pipe->ref_count--;
				if (pipe->ref_count <= 0)
					kfree(pipe);
			}
		}
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
		return -1;

	pipe_buffer_t *buf = (pipe_buffer_t *)kmalloc(sizeof(pipe_buffer_t));
	if (!buf)
		return -1;

	memset(buf, 0, sizeof(pipe_buffer_t));
	buf->ref_count = 2;  // one for each end

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

// Check whether any FD in the table still has the write end of buf open.
static int pipe_write_end_open(file_descriptor_t *fds, const pipe_buffer_t *buf)
{
	for (int i = 0; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_PIPE_WRITE && fds[i].pipe == buf)
			return 1;
	}
	return 0;
}

int virtual_file_system_dup(file_descriptor_t *fds, int oldfd)
{
	if (oldfd < 0 || oldfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -1;
	if (fds[oldfd].type == FD_TYPE_NONE) return -1;

	int newfd = -1;
	for (int i = 3; i < VIRTUAL_FILE_SYSTEM_MAX_FDS; i++) {
		if (fds[i].type == FD_TYPE_NONE) { newfd = i; break; }
	}
	if (newfd < 0) return -1;

	fds[newfd] = fds[oldfd];
	if (fds[newfd].type == FD_TYPE_PIPE_READ ||
	    fds[newfd].type == FD_TYPE_PIPE_WRITE)
		fds[newfd].pipe->ref_count++;

	return newfd;
}

int virtual_file_system_dup2(file_descriptor_t *fds, int oldfd, int newfd)
{
	if (oldfd < 0 || oldfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -1;
	if (newfd < 0 || newfd >= VIRTUAL_FILE_SYSTEM_MAX_FDS) return -1;
	if (fds[oldfd].type == FD_TYPE_NONE) return -1;
	if (oldfd == newfd) return newfd;

	// Close newfd first if it's open.
	if (fds[newfd].type != FD_TYPE_NONE && newfd >= 3)
		virtual_file_system_close(fds, newfd);

	fds[newfd] = fds[oldfd];
	if (fds[newfd].type == FD_TYPE_PIPE_READ ||
	    fds[newfd].type == FD_TYPE_PIPE_WRITE)
		fds[newfd].pipe->ref_count++;

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
static int64_t pipe_read(file_descriptor_t *fds, pipe_buffer_t *buf,
                          char *out, size_t len)
{
	if (len == 0) return 0;
	size_t total = 0;
	while (total < len) {
		if (buf->count == 0) {
			// No data. If write end is closed, signal EOF.
			if (!pipe_write_end_open(fds, buf))
				break;
			// Yield and retry — the writer will eventually produce data.
			scheduler_yield();
			continue;
		}
		out[total] = (char)buf->data[buf->read_pos];
		buf->read_pos = (buf->read_pos + 1) % PIPE_BUFFER_SIZE;
		buf->count--;
		total++;
	}
	return (int64_t)total;
}

// Write up to len bytes to a pipe. Blocks (yield-spin) if the buffer is full.
// Returns -1 if the read end is already closed (broken pipe).
static int64_t pipe_write(file_descriptor_t *fds __attribute__((unused)),
                           pipe_buffer_t *buf,
                           const char *in, size_t len)
{
	if (len == 0) return 0;
	size_t total = 0;
	while (total < len) {
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
	return (int64_t)total;
}

