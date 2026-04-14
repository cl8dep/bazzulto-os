#pragma once
/**
 * @file process.h
 * @brief Bazzulto.System — process management C ABI.
 *
 * Covers process lifecycle (fork/exec/wait/exit), identity (PID/PPID/UID/GID),
 * process groups and sessions, current working directory, file-system helpers,
 * and directory enumeration via bz_getdents64().
 *
 * All path arguments are UTF-8 byte strings; the @p path_len parameter must
 * equal @c strlen(path) — the null terminator is NOT included.
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------
 * Process lifecycle
 * ------------------------------------------------------------------------- */

/**
 * @brief Spawn a child process from a ramfs path.
 *
 * @param path      Absolute path to the executable.
 * @param path_len  Byte length of @p path (not including the null terminator).
 * @return Child PID on success, or a negative errno value on failure.
 */
int64_t bz_spawn(const char *path, size_t path_len);

/**
 * @brief Fork the current process.
 *
 * The child inherits a copy-on-write clone of the parent's address space,
 * open file descriptors, signal handlers, and process group / session.
 *
 * @return 0 in the child, the child PID in the parent, or a negative errno
 *         value on failure.
 */
int64_t bz_fork(void);

/**
 * @brief Replace the current process image (argv only).
 *
 * This variant passes no environment — the new image starts with an empty
 * @c envp.  Prefer bz_execve() when environment propagation is needed.
 *
 * @param path      Absolute path to the new executable.
 * @param path_len  Byte length of @p path (not including the null terminator).
 * @param argv      Flat, null-separated argument buffer
 *                  ("arg0\0arg1\0…argN\0").
 * @param argv_len  Total byte length of @p argv.
 * @return Negative errno value on failure.  Does not return on success.
 */
int64_t bz_exec(const char *path, size_t path_len,
                const char *argv, size_t argv_len);

/**
 * @brief Replace the current process image (argv + envp).
 *
 * POSIX-equivalent of execve().
 *
 * @param path      Absolute path to the new executable.
 * @param path_len  Byte length of @p path.
 * @param argv      Flat, null-separated argument buffer.
 * @param argv_len  Total byte length of @p argv.
 * @param envp      Flat, null-separated environment buffer
 *                  ("KEY=VALUE\0KEY2=VALUE2\0").
 * @param envp_len  Total byte length of @p envp.
 * @return Negative errno value on failure.  Does not return on success.
 */
int64_t bz_execve(const char *path, size_t path_len,
                  const char *argv, size_t argv_len,
                  const char *envp, size_t envp_len);

/**
 * @brief Wait for a child process to exit.
 *
 * @param pid         PID to wait for, or -1 to wait for any child.
 * @param status_out  Pointer where the raw wait status is written (compatible
 *                    with POSIX WIFEXITED / WEXITSTATUS macros). May be NULL.
 * @return The PID of the child that exited, or a negative errno value.
 */
int64_t bz_wait(int32_t pid, int32_t *status_out);

/**
 * @brief Terminate the current process.
 *
 * @param code  Exit status code (0–255).
 * @note Never returns.
 */
__attribute__((noreturn)) void bz_exit(int32_t code);

/* ---------------------------------------------------------------------------
 * Process identity
 * ------------------------------------------------------------------------- */

/**
 * @brief Return the current process PID.
 * @return PID (always positive).
 */
int64_t bz_getpid(void);

/**
 * @brief Return the parent process PID.
 * @return PPID (always positive).
 */
int64_t bz_getppid(void);

/**
 * @brief Return the effective user ID of the calling process.
 * @return UID on success, or a negative errno value.
 */
int64_t bz_getuid(void);

/**
 * @brief Return the effective group ID of the calling process.
 * @return GID on success, or a negative errno value.
 */
int64_t bz_getgid(void);

/* ---------------------------------------------------------------------------
 * Process groups and sessions
 * ------------------------------------------------------------------------- */

/**
 * @brief Return the process group ID of the calling process.
 * @return PGID on success, or a negative errno value.
 */
int64_t bz_getpgrp(void);

/**
 * @brief Set the process group ID of process @p pid.
 *
 * @param pid   Process to change (0 means the calling process).
 * @param pgid  New process group ID (0 means use @p pid as the new PGID).
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_setpgid(int32_t pid, int32_t pgid);

/**
 * @brief Return the session ID of process @p pid.
 *
 * @param pid  Target process (0 means the calling process).
 * @return Session ID on success, or a negative errno value.
 */
int64_t bz_getsid(int32_t pid);

/**
 * @brief Create a new session and set the calling process as its leader.
 *
 * @return The new session ID on success, or a negative errno value.
 */
int64_t bz_setsid(void);

/* ---------------------------------------------------------------------------
 * umask
 * ------------------------------------------------------------------------- */

/**
 * @brief Set the file-creation mode mask and return the previous value.
 *
 * The umask is applied to the @p mode argument of bz_open() (when O_CREAT is
 * set) and bz_mkdir(): @c effective_mode = requested_mode & ~umask.
 *
 * The default umask is @c 0o022.  The umask survives exec().
 *
 * @param mask  New umask (only the low 9 permission bits are used).
 * @return Previous umask value.
 */
uint32_t bz_umask(uint32_t mask);

/* ---------------------------------------------------------------------------
 * Working directory
 * ------------------------------------------------------------------------- */

/**
 * @brief Copy the current working directory path into @p buf.
 *
 * @param buf      Output buffer; written as a null-terminated UTF-8 string.
 * @param buf_len  Size of @p buf in bytes.
 * @return Number of bytes written (including the null terminator) on success,
 *         or a negative errno value on failure (e.g. @c -BZ_ERANGE if buf is
 *         too small).
 */
int64_t bz_getcwd(char *buf, size_t buf_len);

/**
 * @brief Change the current working directory.
 *
 * @param path      New working directory path.
 * @param path_len  Byte length of @p path.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_chdir(const char *path, size_t path_len);

/* ---------------------------------------------------------------------------
 * Directory creation
 * ------------------------------------------------------------------------- */

/**
 * @brief Create a directory.
 *
 * @param path      Path of the new directory.
 * @param path_len  Byte length of @p path.
 * @param mode      Permission bits (e.g. @c 0755); umask is applied.
 * @return 0 on success, or a negative errno value (e.g. @c -BZ_EEXIST if the
 *         path already exists).
 */
int64_t bz_mkdir(const char *path, size_t path_len, uint32_t mode);

/* ---------------------------------------------------------------------------
 * Directory enumeration — bz_getdents64
 * ------------------------------------------------------------------------- */

/**
 * @brief Packed directory entry returned by bz_getdents64().
 *
 * Entries are variable-length and tightly packed in the output buffer.
 * Advance by @c reclen bytes to reach the next entry.
 *
 * @note The @c name field is a null-terminated UTF-8 string embedded at the
 *       end of the record.  Its maximum length is @c reclen - 19 bytes
 *       (including the null terminator).
 */
typedef struct {
    uint64_t inode_number;  /**< Inode number. */
    uint64_t offset;        /**< Implementation-defined directory offset. */
    uint16_t reclen;        /**< Total size of this entry in bytes. */
    uint8_t  type;          /**< File type (one of @c BZ_DT_* values below). */
    char     name[1];       /**< Null-terminated entry name (variable length). */
} bz_dirent64_t;

/** @defgroup dirent_types Directory Entry File Types
 *  @{
 */
#define BZ_DT_UNKNOWN   0   /**< Unknown file type. */
#define BZ_DT_FIFO      1   /**< Named pipe (FIFO). */
#define BZ_DT_CHR       2   /**< Character device. */
#define BZ_DT_DIR       4   /**< Directory. */
#define BZ_DT_BLK       6   /**< Block device. */
#define BZ_DT_REG       8   /**< Regular file. */
#define BZ_DT_LNK      10   /**< Symbolic link. */
#define BZ_DT_SOCK     12   /**< Unix domain socket. */
/** @} */

/**
 * @brief Read directory entries from an open directory file descriptor.
 *
 * Entries are written as a contiguous sequence of variable-length
 * @c bz_dirent64_t records into @p buf.  Iterate by advancing the read
 * pointer by @c entry->reclen bytes after each entry.
 *
 * Returns 0 when all entries have been read (end of directory).
 *
 * @param fd       File descriptor opened on a directory (via bz_open()).
 * @param buf      Output buffer for packed @c bz_dirent64_t records.
 * @param buf_len  Size of @p buf in bytes.
 * @return Number of bytes written into @p buf on success (> 0),
 *         0 at end of directory, or a negative errno value on error.
 */
int64_t bz_getdents64(int32_t fd, void *buf, size_t buf_len);

#ifdef __cplusplus
} /* extern "C" */
#endif
