#pragma once

#include <stddef.h>
#include <stdint.h>
#include "virtual_file_system.h"

// Writable in-memory filesystem driver for //ram: scheme.
//
// Stores up to FS_RAM_MAX_FILES files in a flat inode table. Each inode holds
// a kmalloc'd data buffer that grows on write. Files are identified by their
// full path component (e.g. "/tmp/out.txt").
//
// Virtual devices under //ram:/dev/ are handled as special cases:
//   /dev/null   — reads return 0 bytes, writes are discarded
//   /dev/zero   — reads return zero bytes
//   /dev/random — reads return pseudorandom bytes (CNTPCT_EL0 xorshift)

#define FS_RAM_MAX_FILES    64
#define FS_RAM_INITIAL_SIZE 4096

typedef struct {
    char     name[256];   // full path component, e.g. "/tmp/out.txt"
    uint8_t *data;        // kmalloc'd buffer (NULL for unused slots)
    uint64_t size;        // current content size in bytes
    uint64_t allocated;   // allocated buffer capacity
    int      is_used;     // 1 = live inode, 0 = free slot
} ram_inode_t;

// Initialize the //ram: inode table. Called once at boot after heap_init().
void fs_ram_init(void);

// Create a new file at path (truncates if it already exists).
// Returns a pointer to the inode on success, NULL on failure.
ram_inode_t *fs_ram_creat(const char *path);

// Look up an existing file. Returns the inode pointer, or NULL if not found.
ram_inode_t *fs_ram_lookup(const char *path);

// Write len bytes from buf to inode at the given offset.
// Grows the buffer as needed. Returns bytes written, or -1 on error.
int64_t fs_ram_write(ram_inode_t *inode, const uint8_t *buf,
                     uint64_t offset, uint64_t len);

// Delete a file. Returns 0 on success, -1 if not found.
int fs_ram_unlink(const char *path);

// Return the inode at index (0-based) for enumeration, or NULL if out of range.
// Skips unused slots — callers should iterate until NULL is returned.
ram_inode_t *fs_ram_file_at(int index);

// Scheme driver entry point: open a //ram: path.
// Fills *fd_out and returns 0, or returns -1 on error.
int fs_ram_scheme_open(const char *authority, const char *path,
                       file_descriptor_t *fd_out);

// Scheme driver entry point: create a //ram: file.
int fs_ram_scheme_creat(const char *authority, const char *path,
                        file_descriptor_t *fd_out);

// Scheme driver entry point: unlink a //ram: file.
int fs_ram_scheme_unlink(const char *authority, const char *path);

// Scheme driver entry point: fstat_size for a //ram: fd.
int64_t fs_ram_scheme_fstat_size(const file_descriptor_t *fd);
