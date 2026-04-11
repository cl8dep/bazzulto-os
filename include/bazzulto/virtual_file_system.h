#pragma once

#include <stddef.h>
#include <stdint.h>

// Maximum number of open file descriptors per process.
// fd 0 = stdin, fd 1 = stdout, fd 2 = stderr (reserved).
#define VIRTUAL_FILE_SYSTEM_MAX_FDS 64

// Pipe ring buffer size — bytes available between a write and a read end.
#define PIPE_BUFFER_SIZE 4096

// File descriptor types.
typedef enum {
    FD_TYPE_NONE,       // Slot is unused
    FD_TYPE_CONSOLE,    // stdin/stdout/stderr — backed by UART
    FD_TYPE_FILE,       // Regular file backed by ramfs (read-only)
    FD_TYPE_PIPE_READ,  // Read end of a pipe
    FD_TYPE_PIPE_WRITE, // Write end of a pipe
    FD_TYPE_RAM_FILE,   // Writable file in the //ram: in-memory filesystem
    FD_TYPE_PROC,       // Read-only snapshot buffer from //proc:<pid>/ driver
    FD_TYPE_DEV_NULL,   // /dev/null — reads return 0, writes discarded
    FD_TYPE_DEV_ZERO,   // /dev/zero — reads return zero bytes
    FD_TYPE_DEV_RANDOM, // /dev/random — reads return pseudorandom bytes
    FD_TYPE_DISK_FILE,  // Read-only file on the FAT32 disk (//disk:)
} fd_type_t;

// Forward declarations.
struct ramfs_file;
struct ram_inode;
struct disk_file;       // FAT32 open file state

// Kernel pipe ring buffer. Allocated on the heap; shared via ref_count.
// Both ends of the pipe (read FD and write FD) point to the same pipe_buffer.
// Freed when ref_count drops to zero.
typedef struct {
    uint8_t  data[PIPE_BUFFER_SIZE]; // circular buffer
    uint32_t read_pos;               // index of next byte to read
    uint32_t count;                  // bytes currently available to read
    int      ref_count;              // total number of FDs pointing to this buffer
    int      read_ref_count;         // number of open read-end FDs
                                     // when this reaches 0, writers get broken pipe
} pipe_buffer_t;

// Proc snapshot buffer — pre-filled at open time, read sequentially.
#define PROC_SNAPSHOT_SIZE 512

typedef struct {
    char   buf[PROC_SNAPSHOT_SIZE];
    size_t size;
} proc_snapshot_t;

// A single open file descriptor.
typedef struct {
    fd_type_t type;
    union {
        const struct ramfs_file *file;     // FD_TYPE_FILE
        pipe_buffer_t           *pipe;     // FD_TYPE_PIPE_*
        struct ram_inode        *ram_file; // FD_TYPE_RAM_FILE
        proc_snapshot_t         *proc;     // FD_TYPE_PROC
        struct disk_file        *disk_file;// FD_TYPE_DISK_FILE
    };
    size_t offset; // Current read/write position
} file_descriptor_t;

// Minimal stat structure (fstat syscall result).
struct vfs_stat {
    uint64_t size;    // file size in bytes
    int      type;    // 0 = regular file
};

// Initialize a process's file descriptor table.
// Opens fd 0 (stdin), 1 (stdout), 2 (stderr) as FD_TYPE_CONSOLE.
void virtual_file_system_init_fds(file_descriptor_t *fds);

// Close all open FDs in a process's table (called on process death).
// Decrements pipe ref counts and frees pipe buffers when they reach zero.
void virtual_file_system_close_all_fds(file_descriptor_t *fds);

// Open a file by path. Returns a file descriptor (>= 0) or -1 on error.
// The fd table belongs to the calling process.
int virtual_file_system_open(file_descriptor_t *fds, const char *path);

// Read up to `len` bytes from fd into `buf`.
// Returns bytes read (0 at EOF or write-end-closed), or -1 on error.
// For FD_TYPE_CONSOLE (stdin), reads one byte via UART (blocking).
// For FD_TYPE_PIPE_READ, blocks (yield-spin) until data or write-end closed.
int64_t virtual_file_system_read(file_descriptor_t *fds, int fd, char *buf, size_t len);

// Write up to `len` bytes from `buf` to fd.
// Returns bytes written, or -1 on error.
// For FD_TYPE_CONSOLE (stdout/stderr), writes to UART.
// For FD_TYPE_PIPE_WRITE, blocks (yield-spin) until space or read-end closed.
int64_t virtual_file_system_write(file_descriptor_t *fds, int fd, const char *buf, size_t len);

// Close a file descriptor. Returns 0 on success, -1 on error.
// Cannot close stdin/stdout/stderr (fds 0-2).
int virtual_file_system_close(file_descriptor_t *fds, int fd);

// Seek to a position in a file. Returns new offset, or -1 on error.
// Only valid for FD_TYPE_FILE.
int64_t virtual_file_system_seek(file_descriptor_t *fds, int fd, int64_t offset, int whence);

// Create a kernel pipe. Allocates a pipe_buffer and fills fds[*read_fd_out]
// and fds[*write_fd_out] with the two ends. Returns 0 on success, -1 on error.
int virtual_file_system_pipe(file_descriptor_t *fds,
                             int *read_fd_out, int *write_fd_out);

// Duplicate fd oldfd to the lowest free slot >= 3.
// Returns the new fd on success, -1 on error.
// For pipe FDs, increments the pipe's ref_count.
int virtual_file_system_dup(file_descriptor_t *fds, int oldfd);

// Duplicate fd oldfd to a specific slot newfd, closing newfd first if open.
// Returns newfd on success, -1 on error.
int virtual_file_system_dup2(file_descriptor_t *fds, int oldfd, int newfd);

// Create or truncate a file for writing. Returns fd >= 0, or -1 on error.
// Only writable schemes (//ram:) support creation; //system: returns -1.
int virtual_file_system_creat(file_descriptor_t *fds, const char *path);

// Delete a file by path. Returns 0 on success, -1 on error.
// Only writable schemes support deletion.
int virtual_file_system_unlink(const char *path);

// Fill *stat_out with size information for the given fd.
// Returns 0 on success, -1 on error.
int virtual_file_system_fstat(file_descriptor_t *fds, int fd,
                               struct vfs_stat *stat_out);

// Seek whence constants.
#define VIRTUAL_FILE_SYSTEM_SEEK_SET 0 // Absolute position
#define VIRTUAL_FILE_SYSTEM_SEEK_CUR 1 // Relative to current position
#define VIRTUAL_FILE_SYSTEM_SEEK_END 2 // Relative to end of file
