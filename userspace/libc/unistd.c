#include "unistd.h"
#include "errno.h"

#define BAZZULTO_NO_LEGACY_SYSCALL_NAMES
#include "../library/systemcall.h"

static long syscall_result_to_errno(long result)
{
    if (result >= 0)
        return result;

    errno = (int)(-result);
    return -1;
}

unsigned int sleep(unsigned int seconds)
{
    struct timespec req;
    req.tv_sec  = (long long)seconds;
    req.tv_nsec = 0;

    if (syscall_result_to_errno(sys_nanosleep(&req, (struct timespec *)0)) < 0)
        return seconds;
    return 0;
}

void _exit(int status)
{
    sys_exit(status);
}

ssize_t read(int fd, void *buf, size_t count)
{
    return (ssize_t)syscall_result_to_errno(sys_read(fd, (char *)buf, count));
}

ssize_t write(int fd, const void *buf, size_t count)
{
    return (ssize_t)syscall_result_to_errno(sys_write(fd, (const char *)buf, count));
}

int close(int fd)
{
    return (int)syscall_result_to_errno(sys_close(fd));
}

off_t lseek(int fd, off_t offset, int whence)
{
    return (off_t)syscall_result_to_errno(sys_seek(fd, (int64_t)offset, whence));
}

int dup(int oldfd)
{
    return (int)syscall_result_to_errno(sys_dup(oldfd));
}

int dup2(int oldfd, int newfd)
{
    return (int)syscall_result_to_errno(sys_dup2(oldfd, newfd));
}

int pipe(int fds[2])
{
    return (int)syscall_result_to_errno(sys_pipe(fds));
}

int unlink(const char *path)
{
    return (int)syscall_result_to_errno(sys_unlink(path));
}

pid_t fork(void)
{
    return (pid_t)syscall_result_to_errno(sys_fork());
}

pid_t getpid(void)
{
    return (pid_t)syscall_result_to_errno(sys_getpid());
}

pid_t getppid(void)
{
    return (pid_t)syscall_result_to_errno(sys_getppid());
}

int creat(const char *path, mode_t mode)
{
    (void)mode;
    return (int)syscall_result_to_errno(sys_creat(path));
}

int open(const char *path, int flags, ...)
{
    int access_mode = flags & 0x3;

    if ((flags & O_APPEND) != 0 ||
        (flags & O_DIRECTORY) != 0 ||
        (flags & O_CLOEXEC) != 0 ||
        access_mode == O_RDWR) {
        errno = ENOSYS;
        return -1;
    }

    if (access_mode == O_RDONLY && (flags & (O_CREAT | O_TRUNC)) == 0)
        return (int)syscall_result_to_errno(sys_open(path));

    if (access_mode == O_WRONLY &&
        (flags & O_CREAT) != 0 &&
        (flags & (O_APPEND | O_DIRECTORY | O_CLOEXEC)) == 0)
        return (int)syscall_result_to_errno(sys_creat(path));

    errno = ENOSYS;
    return -1;
}

int execv(const char *path, const char *const argv[])
{
    return (int)syscall_result_to_errno(sys_execv(path, argv));
}

int execve(const char *path, const char *const argv[], const char *const envp[])
{
    if (envp && envp[0]) {
        errno = ENOSYS;
        return -1;
    }
    return execv(path, argv);
}
