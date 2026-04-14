#pragma once
/**
 * @file io.h
 * @brief Bazzulto.IO — file, directory, and stream I/O C ABI.
 *
 * This header covers:
 *   - File open/read/write/seek/close (VFS paths, including FAT32 and tmpfs).
 *   - Stream I/O on stdin (fd 0), stdout (fd 1), stderr (fd 2).
 *   - Directory listing via bz_open() + bz_getdents64() — see process.h.
 *   - Filesystem mount / unmount via bz_mount().
 *   - Path manipulation helpers.
 *
 * Path strings are NUL-terminated UTF-8 C strings.
 *
 * Standard file descriptors:
 *   - 0  stdin  (read)
 *   - 1  stdout (write)
 *   - 2  stderr (write)
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------
 * Open flags (O_* constants) — passed to bz_open()
 * ------------------------------------------------------------------------- */

/** @defgroup open_flags Open Flags
 *  @{
 */
#define BZ_O_RDONLY    0          /**< Open for reading only. */
#define BZ_O_WRONLY    1          /**< Open for writing only. */
#define BZ_O_RDWR      2          /**< Open for reading and writing. */
#define BZ_O_CREAT     0x40       /**< Create the file if it does not exist. */
#define BZ_O_EXCL      0x80       /**< Fail with EEXIST if file already exists (requires O_CREAT). */
#define BZ_O_TRUNC     0x200      /**< Truncate the file to zero on open. */
#define BZ_O_APPEND    0x400      /**< All writes go to the end of the file. */
#define BZ_O_DIRECTORY 0x10000    /**< Fail with ENOTDIR if path is not a directory. */
/** @} */

/* ---------------------------------------------------------------------------
 * Seek whence values — passed to bz_seek()
 * ------------------------------------------------------------------------- */

/** @defgroup seek_whence Seek Whence
 *  @{
 */
#define BZ_SEEK_SET  0  /**< Seek from the beginning of the file. */
#define BZ_SEEK_CUR  1  /**< Seek relative to the current position. */
#define BZ_SEEK_END  2  /**< Seek relative to the end of the file. */
/** @} */

/* ---------------------------------------------------------------------------
 * File stat — returned by bz_fstat()
 * ------------------------------------------------------------------------- */

/**
 * @brief File metadata — analogous to a subset of POSIX struct stat.
 *
 * All size/time fields that the kernel does not yet populate are zero.
 */
typedef struct {
    uint64_t size;          /**< File size in bytes (0 for directories). */
    uint64_t inode_number;  /**< Inode number (filesystem-specific). */
    uint32_t mode;          /**< File type and permission bits (Unix mode). */
    uint32_t link_count;    /**< Number of hard links (1 for most files). */
} bz_stat_t;

/**
 * @brief File type bits within bz_stat_t::mode.
 *  @{
 */
#define BZ_S_IFREG  0100000u  /**< Regular file. */
#define BZ_S_IFDIR  0040000u  /**< Directory. */
#define BZ_S_IFLNK  0120000u  /**< Symbolic link. */
#define BZ_S_IFIFO  0010000u  /**< Named pipe. */
#define BZ_S_IFMT   0170000u  /**< Bitmask for the file type bits. */

/** Test macros — analogous to POSIX S_IS* macros. */
#define BZ_S_ISREG(m)  (((m) & BZ_S_IFMT) == BZ_S_IFREG)
#define BZ_S_ISDIR(m)  (((m) & BZ_S_IFMT) == BZ_S_IFDIR)
#define BZ_S_ISLNK(m)  (((m) & BZ_S_IFMT) == BZ_S_IFLNK)
/** @} */

/* ---------------------------------------------------------------------------
 * Error codes — negative values returned on failure
 * ------------------------------------------------------------------------- */

/** @defgroup errno_codes Errno Values
 *  @{
 */
#define BZ_EPERM    (-1)   /**< Operation not permitted. */
#define BZ_ENOENT   (-2)   /**< No such file or directory. */
#define BZ_ESRCH    (-3)   /**< No such process. */
#define BZ_EINTR    (-4)   /**< Interrupted system call. */
#define BZ_EIO      (-5)   /**< I/O error. */
#define BZ_ENOEXEC  (-8)   /**< Executable format error. */
#define BZ_EBADF    (-9)   /**< Bad file descriptor. */
#define BZ_ECHILD   (-10)  /**< No child processes. */
#define BZ_EAGAIN   (-11)  /**< Resource temporarily unavailable. */
#define BZ_ENOMEM   (-12)  /**< Out of memory. */
#define BZ_EACCES   (-13)  /**< Permission denied. */
#define BZ_EFAULT   (-14)  /**< Bad address. */
#define BZ_EEXIST   (-17)  /**< File exists. */
#define BZ_ENOTDIR  (-20)  /**< Not a directory. */
#define BZ_EINVAL   (-22)  /**< Invalid argument. */
#define BZ_EMFILE   (-24)  /**< Too many open file descriptors. */
#define BZ_ESPIPE   (-29)  /**< Illegal seek (fd is a pipe or FIFO). */
#define BZ_EPIPE    (-32)  /**< Broken pipe. */
#define BZ_ENOSYS   (-38)  /**< Function not implemented. */
/** @} */

/* ---------------------------------------------------------------------------
 * File I/O
 * ------------------------------------------------------------------------- */

/**
 * @brief Open a file or directory.
 *
 * @param path   NUL-terminated UTF-8 path.
 * @param flags  Combination of @c BZ_O_* flags.
 * @param mode   Permission bits applied when @c BZ_O_CREAT is set
 *               (the umask is applied: effective = mode & ~umask).
 *               Pass 0 for read-only opens.
 * @return Non-negative file descriptor on success, or a negative errno value.
 */
int64_t bz_open(const char *path, int32_t flags, uint32_t mode);

/**
 * @brief Close a file descriptor.
 * @param fd  File descriptor to close.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_close(int32_t fd);

/**
 * @brief Read bytes from an open file descriptor.
 *
 * @param fd       Source file descriptor.
 * @param buf      Output buffer.
 * @param buf_len  Maximum bytes to read.
 * @return Number of bytes read (0 = EOF), or a negative errno value.
 */
int64_t bz_read(int32_t fd, void *buf, size_t buf_len);

/**
 * @brief Write bytes to an open file descriptor.
 *
 * @param fd       Destination file descriptor.
 * @param buf      Data to write.
 * @param buf_len  Number of bytes to write.
 * @return Number of bytes written, or a negative errno value.
 */
int64_t bz_write(int32_t fd, const void *buf, size_t buf_len);

/**
 * @brief Reposition the read/write offset of a file descriptor.
 *
 * @param fd      File descriptor.
 * @param offset  Byte offset relative to @p whence.
 * @param whence  @c BZ_SEEK_SET, @c BZ_SEEK_CUR, or @c BZ_SEEK_END.
 * @return New absolute offset from the beginning, or a negative errno value.
 *         Returns @c BZ_ESPIPE if @p fd refers to a pipe.
 */
int64_t bz_seek(int32_t fd, int64_t offset, int32_t whence);

/**
 * @brief Query file metadata by file descriptor.
 *
 * Wraps kernel sys_fstat(26).  The file descriptor must be open; no path
 * lookup is performed.
 *
 * @param fd        Open file descriptor.
 * @param stat_out  Written with file metadata on success.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_fstat(int32_t fd, bz_stat_t *stat_out);

/**
 * @brief Create or truncate a file for writing.
 *
 * Equivalent to @c bz_open(path, BZ_O_WRONLY|BZ_O_CREAT|BZ_O_TRUNC, mode).
 * Matches POSIX creat() which accepts a mode parameter (umask is applied).
 *
 * @param path  NUL-terminated UTF-8 path.
 * @param mode  Permission bits for the newly created file (umask applied).
 * @return Non-negative file descriptor on success, or a negative errno value.
 */
int64_t bz_creat(const char *path, uint32_t mode);

/**
 * @brief Delete a file.
 *
 * @param path  NUL-terminated UTF-8 path.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_unlink(const char *path);

/**
 * @brief Flush pending writes to the underlying storage.
 *
 * @param fd  File descriptor to flush.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_fsync(int32_t fd);

/**
 * @brief Truncate an open file to a given length via its file descriptor.
 *
 * Wraps kernel FTRUNCATE(124).  The file descriptor must be open for writing.
 *
 * @param fd      Open, writable file descriptor.
 * @param length  New file length in bytes.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_ftruncate(int32_t fd, uint64_t length);

/**
 * @brief Truncate a file to a given length by path.
 *
 * Wraps kernel TRUNCATE(58).  The file does not need to be open; the kernel
 * performs a path lookup.  The caller must have write permission on the file.
 *
 * @param path    NUL-terminated UTF-8 path.
 * @param length  New file length in bytes.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_truncate(const char *path, uint64_t length);

/* ---------------------------------------------------------------------------
 * Pipe / dup
 * ------------------------------------------------------------------------- */

/**
 * @brief Create an anonymous pipe.
 *
 * @param fd_pair  Output array of two @c int32_t values: @c fd_pair[0] is the
 *                 read end, @c fd_pair[1] is the write end.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_pipe(int32_t fd_pair[2]);

/**
 * @brief Duplicate a file descriptor to the lowest available fd number.
 * @param fd  File descriptor to duplicate.
 * @return New file descriptor on success, or a negative errno value.
 */
int64_t bz_dup(int32_t fd);

/**
 * @brief Duplicate @p src_fd to @p dst_fd, closing @p dst_fd first if open.
 *
 * @param src_fd  Source file descriptor.
 * @param dst_fd  Desired destination fd number.
 * @return @p dst_fd on success, or a negative errno value.
 */
int64_t bz_dup2(int32_t src_fd, int32_t dst_fd);

/* ---------------------------------------------------------------------------
 * Filesystem — mount and enumeration
 * ------------------------------------------------------------------------- */

/**
 * @brief Mount a filesystem.
 *
 * @param source  NUL-terminated Bazzulto Path Model device path
 *                (e.g. @c "//dev:diskb:1/"), or an empty string for virtual
 *                filesystems.
 * @param target  NUL-terminated mountpoint path (e.g. @c "/home/user").
 * @param fstype  NUL-terminated filesystem type: @c "fat32", @c "bafs",
 *                or @c "tmpfs".
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_mount(const char *source, const char *target, const char *fstype);

/**
 * @brief Enumerate mounted filesystems into @p buf.
 *
 * Buffer format (one record per mount, NUL-terminated fields, newline-separated):
 *   @c "mountpoint source fstype total_blocks free_blocks\n"
 *
 * Pass @c buf = NULL and @c buf_len = 0 to query the required buffer size.
 *
 * @param buf      Output buffer, or NULL to query required size.
 * @param buf_len  Size of @p buf in bytes.
 * @return Total bytes written (or required) on success, or a negative errno value.
 */
int64_t bz_getmounts(void *buf, size_t buf_len);

/* ---------------------------------------------------------------------------
 * Named pipe (FIFO)
 * ------------------------------------------------------------------------- */

/**
 * @brief Create a named pipe (FIFO) at the given path.
 *
 * @param path  NUL-terminated UTF-8 path.
 * @param mode  Permission bits (umask applied).
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_mkfifo(const char *path, uint32_t mode);

/* ---------------------------------------------------------------------------
 * Random data
 * ------------------------------------------------------------------------- */

/** @defgroup getrandom_flags bz_getrandom() Flags
 *  @{
 */
/** Return @c -BZ_EAGAIN instead of blocking if the entropy pool is not yet
 *  seeded.  Without this flag bz_getrandom() blocks until enough entropy is
 *  available. */
#define GRND_NONBLOCK  0x01u
/** @} */

/**
 * @brief Fill @p buf with cryptographically random bytes.
 *
 * @param buf      Output buffer.
 * @param buf_len  Number of bytes to generate.
 * @param flags    0 for normal (blocking) operation, or @c GRND_NONBLOCK to
 *                 return @c -BZ_EAGAIN immediately if the entropy pool is not
 *                 yet ready.
 * @return Number of bytes written on success, or a negative errno value.
 *         With @c GRND_NONBLOCK, returns @c -BZ_EAGAIN if entropy is not ready.
 */
int64_t bz_getrandom(void *buf, size_t buf_len, uint32_t flags);

#ifdef __cplusplus
} /* extern "C" */
#endif
