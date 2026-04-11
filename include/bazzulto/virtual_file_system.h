#pragma once

#include <stddef.h>
#include <stdint.h>

// Maximum number of open file descriptors per process.
// fd 0 = stdin, fd 1 = stdout, fd 2 = stderr (reserved).
#define VIRTUAL_FILE_SYSTEM_MAX_FDS 16

// File descriptor types.
typedef enum {
    FD_TYPE_NONE,    // Slot is unused
    FD_TYPE_CONSOLE, // stdin/stdout/stderr — backed by UART
    FD_TYPE_FILE,    // Regular file backed by ramfs
} fd_type_t;

// Forward declaration — the actual struct is in ramfs.h.
struct ramfs_file;

// A single open file descriptor.
typedef struct {
    fd_type_t type;
    const struct ramfs_file *file; // Non-NULL only for FD_TYPE_FILE
    size_t offset;                 // Current read position within the file
} file_descriptor_t;

// Initialize a process's file descriptor table.
// Opens fd 0 (stdin), 1 (stdout), 2 (stderr) as FD_TYPE_CONSOLE.
void virtual_file_system_init_fds(file_descriptor_t *fds);

// Open a file by path. Returns a file descriptor (>= 0) or -1 on error.
// The fd table belongs to the calling process.
int virtual_file_system_open(file_descriptor_t *fds, const char *path);

// Read up to `len` bytes from fd into `buf`.
// Returns bytes read (0 at EOF), or -1 on error.
// For FD_TYPE_CONSOLE (stdin), reads one byte via UART (blocking).
int64_t virtual_file_system_read(file_descriptor_t *fds, int fd, char *buf, size_t len);

// Write up to `len` bytes from `buf` to fd.
// Returns bytes written, or -1 on error.
// For FD_TYPE_CONSOLE (stdout/stderr), writes to UART.
int64_t virtual_file_system_write(file_descriptor_t *fds, int fd, const char *buf, size_t len);

// Close a file descriptor. Returns 0 on success, -1 on error.
// Cannot close stdin/stdout/stderr.
int virtual_file_system_close(file_descriptor_t *fds, int fd);

// Seek to a position in a file. Returns new offset, or -1 on error.
// Only valid for FD_TYPE_FILE.
int64_t virtual_file_system_seek(file_descriptor_t *fds, int fd, int64_t offset, int whence);

// Seek whence constants.
#define VIRTUAL_FILE_SYSTEM_SEEK_SET 0 // Absolute position
#define VIRTUAL_FILE_SYSTEM_SEEK_CUR 1 // Relative to current position
#define VIRTUAL_FILE_SYSTEM_SEEK_END 2 // Relative to end of file
