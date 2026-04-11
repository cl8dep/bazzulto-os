#pragma once

// POSIX-facing userspace API layered on top of Bazzulto raw syscalls.

#include <stddef.h>
#include <stdint.h>

typedef int     pid_t;
typedef int     mode_t;
typedef int64_t ssize_t;
typedef int64_t off_t;

#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

// Minimal open(2) flags currently supported by libc.
#define O_RDONLY    0x0000
#define O_WRONLY    0x0001
#define O_RDWR      0x0002
#define O_CREAT     0x0040
#define O_TRUNC     0x0200
#define O_APPEND    0x0400
#define O_CLOEXEC   0x80000
#define O_DIRECTORY 0x10000

// fcntl(2) subset.
#define F_GETFD   1
#define F_SETFD   2
#define FD_CLOEXEC 1

void _exit(int status) __attribute__((noreturn));

pid_t fork(void);
pid_t getpid(void);
pid_t getppid(void);

ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int     close(int fd);
off_t   lseek(int fd, off_t offset, int whence);
int     dup(int oldfd);
int     dup2(int oldfd, int newfd);
int     pipe(int fds[2]);
int     unlink(const char *path);

int     open(const char *path, int flags, ...);
int     creat(const char *path, mode_t mode);

int     execv(const char *path, const char *const argv[]);
int     execve(const char *path, const char *const argv[], const char *const envp[]);

unsigned int sleep(unsigned int seconds);
