// //system: scheme driver — read-only wrapper around the boot ramfs.
//
// Maps every path through ramfs_lookup(). The //system: authority is ignored
// (all files live in a flat namespace keyed by their full path).
//
// Example:
//   //system:/bin/hello  →  ramfs_lookup("/bin/hello")
//   /bin/hello           →  mount table routes to this driver with path="/bin/hello"

#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/ramfs.h"
#include "errno.h"

int fs_system_open(const char *authority __attribute__((unused)),
                   const char *path,
                   file_descriptor_t *fd_out)
{
    const struct ramfs_file *file = ramfs_lookup(path);
    if (!file)
        return -ENOENT;

    fd_out->type   = FD_TYPE_FILE;
    fd_out->file   = file;
    fd_out->offset = 0;
    return 0;
}

// //system: is read-only — creat and unlink are not supported.
int fs_system_creat(const char *authority __attribute__((unused)),
                    const char *path __attribute__((unused)),
                    file_descriptor_t *fd_out __attribute__((unused)))
{
    return -EROFS;
}

int fs_system_unlink(const char *authority __attribute__((unused)),
                     const char *path __attribute__((unused)))
{
    return -EROFS;
}

int64_t fs_system_fstat_size(const file_descriptor_t *fd)
{
    if (fd->type != FD_TYPE_FILE || !fd->file)
        return -EBADF;
    return (int64_t)fd->file->size;
}

// Exported driver descriptor — used by vfs_scheme_init().
const vfs_scheme_driver_t fs_system_driver = {
    .scheme      = "system",
    .open        = fs_system_open,
    .creat       = fs_system_creat,
    .unlink      = fs_system_unlink,
    .fstat_size  = fs_system_fstat_size,
};
