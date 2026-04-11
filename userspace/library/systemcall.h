#pragma once

// User-space system call interface for Bazzulto OS.
// Each function is a thin wrapper around the SVC instruction.
// Arguments follow the AAPCS64 calling convention (x0-x5),
// which matches the kernel's syscall register convention.

#include <stddef.h>
#include <stdint.h>

// Terminate the calling process. Does not return.
void exit(int status) __attribute__((noreturn));

// Write up to `len` bytes from `buf` to file descriptor `fd`.
// Returns bytes written, or -1 on error.
int64_t write(int fd, const char *buf, size_t len);

// Read up to `len` bytes into `buf` from file descriptor `fd`.
// Returns bytes read (0 at EOF), or -1 on error.
int64_t read(int fd, char *buf, size_t len);

// Voluntarily yield the CPU to another process.
int yield(void);

// Open a file by path. Returns a file descriptor (>= 0), or -1 on error.
int open(const char *path);

// Close a file descriptor. Returns 0 on success, -1 on error.
int close(int fd);

// Reposition the read offset of a file descriptor.
// `whence`: 0 = from start, 1 = from current, 2 = from end.
// Returns the new offset, or -1 on error.
int64_t seek(int fd, int64_t offset, int whence);

// Spawn a new process from the executable at `path`.
// `argv` is a NULL-terminated array of argument strings.
// argv[0] should be the program name.
// Pass NULL for no arguments.
// Returns the child PID (> 0), or -1 on error.
int spawn(const char *path, const char *const argv[]);

// List a file in the filesystem by index.
// Copies the file name into `name_buf` (null-terminated, up to `buf_len` bytes).
// Returns the file size in bytes, or -1 if the index is out of range.
int64_t list(int index, char *name_buf, size_t buf_len);

// Block until the process with the given PID exits.
// Returns the child's exit status (>= 0), or -1 if the PID is not found.
int64_t wait(int pid);

// Create a kernel pipe. Fills fds[0] (read end) and fds[1] (write end).
// Returns 0 on success, -1 on error.
int pipe(int fds[2]);

// Duplicate a file descriptor to the lowest free slot >= 3.
// Returns the new fd, or -1 on error.
int dup(int oldfd);

// Duplicate oldfd to newfd, closing newfd first if open.
// Returns newfd on success, -1 on error.
int dup2(int oldfd, int newfd);

// Allocate `length` bytes of anonymous, zeroed memory.
// Returns a user virtual address on success, or (void *)-1 on error.
void *mmap(size_t length);

// Release memory previously returned by mmap.
// Returns 0 on success, -1 if the address is not a known mmap allocation.
int munmap(void *addr);

// Fork the calling process.
// Returns the child PID (> 0) in the parent, 0 in the child, -1 on error.
int fork(void);

// Replace the current process image with the ELF at `path`.
// On success: does not return — execution restarts at the new entry point.
// On failure: returns -1.
int exec(const char *path);
