#include "../../include/bazzulto/virtual_file_system.h"
#include "../../include/bazzulto/ramfs.h"
#include "../../include/bazzulto/uart.h"

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
		buf[0] = uart_getc();
		return 1;
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
		return -1;
	if (fds[fd].type == FD_TYPE_NONE)
		return -1;

	if (fds[fd].type == FD_TYPE_CONSOLE) {
		// Only stdout (fd 1) and stderr (fd 2) support writing.
		if (fd != 1 && fd != 2)
			return -1;
		for (size_t i = 0; i < len; i++)
			uart_putc(buf[i]);
		return (int64_t)len;
	}

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
