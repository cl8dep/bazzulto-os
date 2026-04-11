#pragma once

// Error codes — C11 §7.5, POSIX.1
// errno is a per-process global set by syscalls and libc functions on failure.

extern int errno;

// C11 mandatory codes
#define EDOM    1
#define EILSEQ  2
#define ERANGE  3
#define BZ_ERR_MATH_DOMAIN      EDOM    // Math argument out of domain
#define BZ_ERR_ILLEGAL_SEQUENCE EILSEQ  // Illegal byte sequence
#define BZ_ERR_OUT_OF_RANGE     ERANGE  // Result out of range

// POSIX codes used by Bazzulto syscalls
#define EPERM   4
#define ENOENT  5
#define EBADF   6
#define ENOMEM  7
#define EACCES  8
#define EEXIST  9
#define ENOTDIR 10
#define EISDIR  11
#define EINVAL  12
#define EMFILE  13
#define ENOSPC  14
#define ESPIPE  15
#define ENOSYS  16
#define BZ_ERR_NOT_PERMITTED    EPERM   // Operation not permitted
#define BZ_ERR_FILE_NOT_FOUND   ENOENT  // No such file or directory
#define BZ_ERR_BAD_FD           EBADF   // Bad file descriptor
#define BZ_ERR_OUT_OF_MEMORY    ENOMEM  // Out of memory
#define BZ_ERR_ACCESS_DENIED    EACCES  // Permission denied
#define BZ_ERR_FILE_EXISTS      EEXIST  // File already exists
#define BZ_ERR_NOT_A_DIRECTORY  ENOTDIR // Not a directory
#define BZ_ERR_IS_A_DIRECTORY   EISDIR  // Is a directory
#define BZ_ERR_INVALID_ARGUMENT EINVAL  // Invalid argument
#define BZ_ERR_TOO_MANY_FDS     EMFILE  // Too many open file descriptors
#define BZ_ERR_NO_SPACE         ENOSPC  // No space left
#define BZ_ERR_NOT_SEEKABLE     ESPIPE  // Seek on non-seekable fd
#define BZ_ERR_NOT_IMPLEMENTED  ENOSYS  // Function not implemented
