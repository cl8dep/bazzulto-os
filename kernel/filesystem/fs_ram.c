// //ram: scheme driver — writable in-memory filesystem.
//
// Files are stored in a flat inode table. Each inode holds a kmalloc'd buffer
// that grows on demand. Virtual devices under /dev/ are handled as special
// cases that return synthetic fd types (FD_TYPE_DEV_NULL etc.).

#include "../../include/bazzulto/fs_ram.h"
#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/heap.h"
#include "errno.h"
#include <string.h>
#include <stddef.h>

// Cast ram_inode_t* from the forward-declared struct ram_inode* used in VFS.
// Both names refer to the same layout; we use typedef ram_inode_t in fs_ram.h
// and struct ram_inode as a forward decl in virtual_file_system.h.
// No extra typedef needed here — we use ram_inode_t throughout.

static ram_inode_t inode_table[FS_RAM_MAX_FILES];

void fs_ram_init(void)
{
    memset(inode_table, 0, sizeof(inode_table));
}

// Find a free inode slot. Returns NULL if the table is full.
static ram_inode_t *alloc_inode(void)
{
    for (int i = 0; i < FS_RAM_MAX_FILES; i++) {
        if (!inode_table[i].is_used)
            return &inode_table[i];
    }
    return NULL;
}

ram_inode_t *fs_ram_lookup(const char *path)
{
    for (int i = 0; i < FS_RAM_MAX_FILES; i++) {
        if (inode_table[i].is_used &&
            strcmp(inode_table[i].name, path) == 0)
            return &inode_table[i];
    }
    return NULL;
}

ram_inode_t *fs_ram_creat(const char *path)
{
    // Truncate if already exists.
    ram_inode_t *existing = fs_ram_lookup(path);
    if (existing) {
        existing->size = 0;
        return existing;
    }

    ram_inode_t *inode = alloc_inode();
    if (!inode)
        return NULL;

    // Copy path (truncate to fit).
    size_t path_len = strlen(path);
    if (path_len >= sizeof(inode->name))
        path_len = sizeof(inode->name) - 1;
    memcpy(inode->name, path, path_len);
    inode->name[path_len] = '\0';

    inode->data      = (uint8_t *)kmalloc(FS_RAM_INITIAL_SIZE);
    inode->allocated = inode->data ? FS_RAM_INITIAL_SIZE : 0;
    inode->size      = 0;
    inode->is_used   = 1;
    return inode;
}

int64_t fs_ram_write(ram_inode_t *inode, const uint8_t *buf,
                     uint64_t offset, uint64_t len)
{
    if (!inode || !buf || len == 0)
        return 0;

    uint64_t needed = offset + len;
    if (needed > inode->allocated) {
        // Grow — at least double, at least needed.
        uint64_t new_capacity = inode->allocated * 2;
        if (new_capacity < needed)
            new_capacity = needed;
        uint8_t *new_buf = (uint8_t *)kmalloc((size_t)new_capacity);
        if (!new_buf)
            return -ENOMEM;
        if (inode->data && inode->size > 0)
            memcpy(new_buf, inode->data, (size_t)inode->size);
        if (inode->data)
            kfree(inode->data);
        inode->data      = new_buf;
        inode->allocated = new_capacity;
    }

    memcpy(inode->data + offset, buf, (size_t)len);
    if (offset + len > inode->size)
        inode->size = offset + len;
    return (int64_t)len;
}

int fs_ram_unlink(const char *path)
{
    ram_inode_t *inode = fs_ram_lookup(path);
    if (!inode)
        return -ENOENT;
    if (inode->data)
        kfree(inode->data);
    memset(inode, 0, sizeof(*inode));
    return 0;
}

ram_inode_t *fs_ram_file_at(int index)
{
    int count = 0;
    for (int i = 0; i < FS_RAM_MAX_FILES; i++) {
        if (inode_table[i].is_used) {
            if (count == index)
                return &inode_table[i];
            count++;
        }
    }
    return NULL;
}

// ---------------------------------------------------------------------------
// Virtual devices
// ---------------------------------------------------------------------------

static int is_dev_null(const char *path)
{
    return strcmp(path, "/dev/null") == 0;
}

static int is_dev_zero(const char *path)
{
    return strcmp(path, "/dev/zero") == 0;
}

static int is_dev_random(const char *path)
{
    return strcmp(path, "/dev/random") == 0;
}

// ---------------------------------------------------------------------------
// Scheme driver entry points
// ---------------------------------------------------------------------------

int fs_ram_scheme_open(const char *authority __attribute__((unused)),
                       const char *path,
                       file_descriptor_t *fd_out)
{
    if (is_dev_null(path)) {
        fd_out->type = FD_TYPE_DEV_NULL;
        fd_out->ram_file = NULL;
        fd_out->offset = 0;
        return 0;
    }
    if (is_dev_zero(path)) {
        fd_out->type = FD_TYPE_DEV_ZERO;
        fd_out->ram_file = NULL;
        fd_out->offset = 0;
        return 0;
    }
    if (is_dev_random(path)) {
        fd_out->type = FD_TYPE_DEV_RANDOM;
        fd_out->ram_file = NULL;
        fd_out->offset = 0;
        return 0;
    }

    ram_inode_t *inode = fs_ram_lookup(path);
    if (!inode)
        return -ENOENT;

    fd_out->type     = FD_TYPE_RAM_FILE;
    fd_out->ram_file = (struct ram_inode *)inode;
    fd_out->offset   = 0;
    return 0;
}

int fs_ram_scheme_creat(const char *authority __attribute__((unused)),
                        const char *path,
                        file_descriptor_t *fd_out)
{
    // Virtual devices: creat behaves like open (no truncation needed).
    if (is_dev_null(path)) {
        fd_out->type = FD_TYPE_DEV_NULL;
        fd_out->ram_file = NULL;
        fd_out->offset = 0;
        return 0;
    }
    if (is_dev_zero(path) || is_dev_random(path))
        return -EACCES;  // not writable

    ram_inode_t *inode = fs_ram_creat(path);
    if (!inode)
        return -ENOSPC;

    fd_out->type     = FD_TYPE_RAM_FILE;
    fd_out->ram_file = (struct ram_inode *)inode;
    fd_out->offset   = 0;
    return 0;
}

int fs_ram_scheme_unlink(const char *authority __attribute__((unused)),
                         const char *path)
{
    return fs_ram_unlink(path);
}

int64_t fs_ram_scheme_fstat_size(const file_descriptor_t *fd)
{
    if (fd->type == FD_TYPE_RAM_FILE && fd->ram_file)
        return (int64_t)((ram_inode_t *)fd->ram_file)->size;
    if (fd->type == FD_TYPE_DEV_NULL ||
        fd->type == FD_TYPE_DEV_ZERO ||
        fd->type == FD_TYPE_DEV_RANDOM)
        return 0;
    return -EBADF;
}

const vfs_scheme_driver_t fs_ram_driver = {
    .scheme     = "ram",
    .open       = fs_ram_scheme_open,
    .creat      = fs_ram_scheme_creat,
    .unlink     = fs_ram_scheme_unlink,
    .fstat_size = fs_ram_scheme_fstat_size,
};
