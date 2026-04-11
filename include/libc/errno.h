#pragma once

// Error codes shared between kernel and userspace libc.
// The numeric values are stable within Bazzulto and are used by the kernel
// to return -errno style failures to EL0. Userspace libc translates those
// negative return values into errno + -1 for POSIX-style APIs.

extern int errno;

// C11 mandatory codes
#define EDOM      1
#define EILSEQ    2
#define ERANGE    3

// POSIX-style codes used by Bazzulto
#define EPERM     4
#define ENOENT    5
#define EBADF     6
#define ENOMEM    7
#define EACCES    8
#define EEXIST    9
#define ENOTDIR   10
#define EISDIR    11
#define EINVAL    12
#define EMFILE    13
#define ENOSPC    14
#define ESPIPE    15
#define ENOSYS    16
#define EINTR     17
#define ELOOP     18
#define EROFS     19
#define ENOTEMPTY 20
#define EXDEV     21
#define EFAULT    22
#define ECHILD    23
#define EAGAIN    24
#define ESRCH     25
