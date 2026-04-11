#pragma once

// Raw user-space syscall interface for Bazzulto OS.
// These functions map 1:1 to SVC instructions and return the kernel value
// directly, including negative errno-style failures.
//
// Public applications should prefer userspace/libc headers. This header exists
// for libc internals and for legacy Bazzulto tools that still talk to the raw
// syscall layer directly.

#include <stddef.h>
#include <stdint.h>

void    sys_exit(int status) __attribute__((noreturn));
int64_t sys_write(int fd, const char *buf, size_t len);
int64_t sys_read(int fd, char *buf, size_t len);
int     sys_yield(void);
int     sys_open(const char *path);
int     sys_close(int fd);
int64_t sys_seek(int fd, int64_t offset, int whence);
int     sys_spawn(const char *path, const char *const argv[]);
int64_t sys_list(int index, char *name_buf, size_t buf_len);
int64_t sys_wait(int pid);
int     sys_pipe(int fds[2]);
int     sys_dup(int oldfd);
int     sys_dup2(int oldfd, int newfd);
void   *sys_mmap(size_t length);
int     sys_munmap(void *addr);
int     sys_fork(void);
int     sys_exec(const char *path);
int     sys_execv(const char *path, const char *const argv[]);
int     sys_getpid(void);
int     sys_getppid(void);

struct timespec {
    long long tv_sec;
    long long tv_nsec;
};

#define CLOCK_REALTIME  0
#define CLOCK_MONOTONIC 1

int sys_clock_gettime(int clock_id, struct timespec *tp);
int sys_nanosleep(const struct timespec *req, struct timespec *rem);

#define SIGHUP   1
#define SIGINT   2
#define SIGQUIT  3
#define SIGKILL  9
#define SIGTERM 15
#define SIGCHLD 17
#define SIGSTOP 19

#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)

int sys_sigaction(int signum, void (*handler)(int));
int sys_kill(int pid, int signum);
int sys_creat(const char *path);
int sys_unlink(const char *path);

struct stat {
    unsigned long long size;
    int                type;
};

int  sys_fstat(int fd, struct stat *st);
void sys_set_terminal_foreground_pid(int pid);

struct disk_info {
    unsigned long long capacity_sectors;
    unsigned long long free_clusters;
    unsigned long long total_clusters;
    unsigned long long bytes_per_cluster;
    int                ready;
};

int sys_disk_info(struct disk_info *info);

#ifndef BAZZULTO_NO_LEGACY_SYSCALL_NAMES
#define exit(status) sys_exit((status))
#define write(fd, buf, len) sys_write((fd), (buf), (len))
#define read(fd, buf, len) sys_read((fd), (buf), (len))
#define yield() sys_yield()
#define open(path) sys_open((path))
#define close(fd) sys_close((fd))
#define seek(fd, offset, whence) sys_seek((fd), (offset), (whence))
#define spawn(path, argv) sys_spawn((path), (argv))
#define list(index, name_buf, buf_len) sys_list((index), (name_buf), (buf_len))
#define wait(pid) sys_wait((pid))
#define pipe(fds) sys_pipe((fds))
#define dup(oldfd) sys_dup((oldfd))
#define dup2(oldfd, newfd) sys_dup2((oldfd), (newfd))
#define mmap(length) sys_mmap((length))
#define munmap(addr) sys_munmap((addr))
#define fork() sys_fork()
#define exec(path) sys_exec((path))
#define execv(path, argv) sys_execv((path), (argv))
#define getpid() sys_getpid()
#define getppid() sys_getppid()
#define clock_gettime(clock_id, tp) sys_clock_gettime((clock_id), (tp))
#define nanosleep(req, rem) sys_nanosleep((req), (rem))
#define sigaction(signum, handler) sys_sigaction((signum), (handler))
#define kill(pid, signum) sys_kill((pid), (signum))
#define creat(path) sys_creat((path))
#define unlink(path) sys_unlink((path))
#define fstat(fd, st) sys_fstat((fd), (st))
#define set_terminal_foreground_pid(pid) sys_set_terminal_foreground_pid((pid))
#define disk_info(info) sys_disk_info((info))
#endif
