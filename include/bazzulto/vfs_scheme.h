#pragma once

#include <stddef.h>
#include <stdint.h>
#include "virtual_file_system.h"

// Scheme-based VFS router for Bazzulto OS.
//
// Canonical path form:  //scheme:authority/path
//   e.g.  //system:/bin/hello
//         //ram:/tmp/out.txt
//         //proc:42/status
//
// Unix legacy form:  /prefix/rest
//   Mapped to a scheme via the static mount table (see vfs_scheme.c).
//   No fallback — unknown prefixes return -1.
//
// Both forms are first-class. New kernel and userspace code should prefer
// the canonical //scheme: form; /prefix/ paths exist for backward compatibility.

// Per-scheme driver. Registered at boot in vfs_scheme_init().
typedef struct {
    const char *scheme;  // e.g. "system", "ram", "proc"

    // Open an existing file. authority may be empty ("") for schemes that don't
    // use it (system, ram). Returns 0 and fills *fd_out on success, -1 on error.
    int (*open)(const char *authority, const char *path,
                file_descriptor_t *fd_out);

    // Create or truncate a file for writing. Returns 0 + fills *fd_out, or -1.
    // NULL if the scheme is read-only (e.g. //system:).
    int (*creat)(const char *authority, const char *path,
                 file_descriptor_t *fd_out);

    // Delete a file. Returns 0 on success, -1 on error or if read-only.
    // NULL if the scheme is read-only.
    int (*unlink)(const char *authority, const char *path);

    // Return the size of the open file referenced by fd_out. Returns size >= 0
    // or -1 on error.
    int64_t (*fstat_size)(const file_descriptor_t *fd);
} vfs_scheme_driver_t;

// Initialize the scheme router and register all built-in drivers.
// Must be called after heap_init() (//ram: driver uses kmalloc).
void vfs_scheme_init(void);

// Open a file by path (canonical or Unix form).
// Returns 0 and fills *fd_out on success, -1 on error.
int vfs_scheme_open(const char *path, file_descriptor_t *fd_out);

// Create or truncate a file for writing.
// Returns 0 and fills *fd_out on success, -1 on error or read-only scheme.
int vfs_scheme_creat(const char *path, file_descriptor_t *fd_out);

// Delete a file. Returns 0 on success, -1 on error or read-only scheme.
int vfs_scheme_unlink(const char *path);

// Return the size of an open file. Returns size >= 0 or -1 on error.
int64_t vfs_scheme_fstat_size(const file_descriptor_t *fd);
