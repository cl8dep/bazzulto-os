#pragma once

#include <stddef.h>
#include <stdint.h>

// Maximum number of files the ramfs can hold.
// This is a boot-time filesystem — files are registered once and never removed.
#define RAMFS_MAX_FILES 32

// Maximum file name length including null terminator.
#define RAMFS_MAX_NAME  64

// A single file entry in the ramfs.
// The data pointer references memory in kernel space (typically the .user_text
// section or a kmalloc'd buffer). The ramfs does not own or copy the data.
struct ramfs_file {
    char name[RAMFS_MAX_NAME]; // File path (e.g., "/bin/echo")
    const uint8_t *data;       // Pointer to file content in kernel memory
    size_t size;               // File size in bytes
};

// Initialize the ramfs (zeroes the file table).
void ramfs_init(void);

// Register a file in the ramfs.
// The data pointer must remain valid for the lifetime of the kernel.
// Returns 0 on success, -1 if the table is full or name is too long.
int ramfs_register(const char *name, const void *data, size_t size);

// Look up a file by exact path.
// Returns a pointer to the file entry, or NULL if not found.
const struct ramfs_file *ramfs_lookup(const char *name);

// Return the number of registered files.
int ramfs_file_count(void);

// Return a pointer to the file entry at the given index (0-based).
// Returns NULL if index is out of range.
const struct ramfs_file *ramfs_file_at(int index);
