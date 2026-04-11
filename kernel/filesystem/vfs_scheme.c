// Scheme-based VFS router for Bazzulto OS.
//
// Canonical form:  //scheme:authority/path
// Unix legacy:     /prefix/rest  (mapped via static mount table)
//
// No fallback — unknown /prefix/ paths return -1.

#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/fs_ram.h"
#include <string.h>
#include <stddef.h>

// ---------------------------------------------------------------------------
// Scheme driver table — forward-declared from their respective .c files.
// ---------------------------------------------------------------------------

extern const vfs_scheme_driver_t fs_system_driver;
extern const vfs_scheme_driver_t fs_ram_driver;
extern const vfs_scheme_driver_t fs_proc_driver;
extern const vfs_scheme_driver_t fs_disk_driver;

static const vfs_scheme_driver_t *scheme_drivers[] = {
    &fs_system_driver,
    &fs_ram_driver,
    &fs_proc_driver,
    &fs_disk_driver,
};
#define NUM_SCHEME_DRIVERS 4

// ---------------------------------------------------------------------------
// Unix mount table — maps /prefix/ to a scheme + path transformation.
// ---------------------------------------------------------------------------

typedef struct {
    const char *unix_prefix;   // e.g. "/bin/"
    const char *scheme;        // e.g. "system"
    const char *authority;     // e.g. ""
    // If remap_path is non-NULL, the unix path is passed verbatim to the driver.
    // If NULL, the path is also passed verbatim (prefix is NOT stripped).
    int strip_prefix;          // 0 = pass full unix path; 1 = strip prefix
} mount_entry_t;

// Static mount table.
// All entries are checked in order; the first matching prefix wins.
// /proc/ is special: the PID comes from the next path component, so we set
// authority dynamically in vfs_scheme_parse_unix().
static const mount_entry_t mount_table[] = {
    { "/bin/",  "system", "", 0 },
    { "/lib/",  "system", "", 0 },
    { "/etc/",  "system", "", 0 },
    { "/usr/",  "system", "", 0 },
    { "/tmp/",  "ram",    "", 0 },
    { "/run/",  "ram",    "", 0 },
    { "/var/",  "ram",    "", 0 },
    { "/dev/",  "ram",    "", 0 },  // /dev/ lives under //ram:/dev/
    { "/proc/", "proc",   "", 0 },  // authority parsed dynamically
    { "/mnt/",  "disk",   "", 0 },  // FAT32 disk mounted at /mnt/
};
#define MOUNT_TABLE_ENTRIES ((int)(sizeof(mount_table) / sizeof(mount_table[0])))

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static const vfs_scheme_driver_t *find_driver(const char *scheme)
{
    for (int i = 0; i < NUM_SCHEME_DRIVERS; i++) {
        if (strcmp(scheme_drivers[i]->scheme, scheme) == 0)
            return scheme_drivers[i];
    }
    return NULL;
}

// ---------------------------------------------------------------------------
// Parse a path into (scheme, authority, sub-path) and invoke the given
// driver operation. `op` selects open / creat / unlink.
// ---------------------------------------------------------------------------

typedef enum { VFS_OP_OPEN, VFS_OP_CREAT, VFS_OP_UNLINK } vfs_op_t;

// Result union for dispatch.
typedef struct {
    file_descriptor_t fd;   // valid on open/creat success
    int               rc;   // 0 = success, -1 = error (unlink)
} vfs_dispatch_result_t;

static int dispatch(const char *path, vfs_op_t op,
                    file_descriptor_t *fd_out)
{
    char scheme_buf[32];
    char authority_buf[64];
    const char *sub_path;

    // ---- Canonical form: //scheme:authority/path ----
    if (path[0] == '/' && path[1] == '/') {
        const char *start = path + 2;  // skip "//"

        // Find ':' to extract scheme.
        const char *colon = start;
        while (*colon && *colon != ':')
            colon++;
        if (*colon != ':')
            return -1;

        size_t scheme_len = (size_t)(colon - start);
        if (scheme_len == 0 || scheme_len >= sizeof(scheme_buf))
            return -1;
        memcpy(scheme_buf, start, scheme_len);
        scheme_buf[scheme_len] = '\0';

        // Authority is between ':' and the next '/'.
        const char *auth_start = colon + 1;
        const char *slash = auth_start;
        while (*slash && *slash != '/')
            slash++;

        size_t auth_len = (size_t)(slash - auth_start);
        if (auth_len >= sizeof(authority_buf))
            return -1;
        memcpy(authority_buf, auth_start, auth_len);
        authority_buf[auth_len] = '\0';

        // Sub-path is from '/' onward (may be empty → just "/").
        sub_path = (*slash == '/') ? slash : "/";

        const vfs_scheme_driver_t *drv = find_driver(scheme_buf);
        if (!drv)
            return -1;

        switch (op) {
        case VFS_OP_OPEN:
            return drv->open(authority_buf, sub_path, fd_out);
        case VFS_OP_CREAT:
            if (!drv->creat) return -1;
            return drv->creat(authority_buf, sub_path, fd_out);
        case VFS_OP_UNLINK:
            if (!drv->unlink) return -1;
            return drv->unlink(authority_buf, sub_path);
        }
        return -1;
    }

    // ---- Unix legacy form: /prefix/rest ----
    for (int i = 0; i < MOUNT_TABLE_ENTRIES; i++) {
        const mount_entry_t *entry = &mount_table[i];
        size_t prefix_len = strlen(entry->unix_prefix);
        if (strncmp(path, entry->unix_prefix, prefix_len) != 0)
            continue;

        // Matched. Build authority for /proc/ paths.
        if (strcmp(entry->scheme, "proc") == 0) {
            // /proc/<pid>/<rest> — extract pid as authority, /rest as sub-path.
            const char *pid_start = path + prefix_len;  // character after "/proc/"
            const char *pid_end   = pid_start;
            while (*pid_end && *pid_end != '/')
                pid_end++;

            size_t pid_len = (size_t)(pid_end - pid_start);
            if (pid_len == 0 || pid_len >= sizeof(authority_buf))
                return -1;
            memcpy(authority_buf, pid_start, pid_len);
            authority_buf[pid_len] = '\0';

            sub_path = (*pid_end == '/') ? pid_end : "/";
        } else {
            // For all other schemes the authority is empty and we pass the
            // full unix path (no stripping). This lets ramfs_lookup("/bin/hello")
            // work unchanged, and lets //ram: use the full path as the inode key.
            authority_buf[0] = '\0';
            sub_path = path;
        }

        const vfs_scheme_driver_t *drv = find_driver(entry->scheme);
        if (!drv)
            return -1;

        switch (op) {
        case VFS_OP_OPEN:
            return drv->open(authority_buf, sub_path, fd_out);
        case VFS_OP_CREAT:
            if (!drv->creat) return -1;
            return drv->creat(authority_buf, sub_path, fd_out);
        case VFS_OP_UNLINK:
            if (!drv->unlink) return -1;
            return drv->unlink(authority_buf, sub_path);
        }
        return -1;
    }

    // No matching mount entry — do not fall back.
    return -1;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

void vfs_scheme_init(void)
{
    fs_ram_init();
    // fat32_init() is called later in main.c, after hal_platform_init() and
    // hal_disk_init() have had a chance to enumerate virtio devices.
}

int vfs_scheme_open(const char *path, file_descriptor_t *fd_out)
{
    return dispatch(path, VFS_OP_OPEN, fd_out);
}

int vfs_scheme_creat(const char *path, file_descriptor_t *fd_out)
{
    return dispatch(path, VFS_OP_CREAT, fd_out);
}

int vfs_scheme_unlink(const char *path)
{
    return dispatch(path, VFS_OP_UNLINK, NULL);
}

int64_t vfs_scheme_fstat_size(const file_descriptor_t *fd)
{
    switch (fd->type) {
    case FD_TYPE_FILE:
        return find_driver("system") ?
               find_driver("system")->fstat_size(fd) : -1;
    case FD_TYPE_RAM_FILE:
    case FD_TYPE_DEV_NULL:
    case FD_TYPE_DEV_ZERO:
    case FD_TYPE_DEV_RANDOM:
        return find_driver("ram") ?
               find_driver("ram")->fstat_size(fd) : -1;
    case FD_TYPE_PROC:
        return find_driver("proc") ?
               find_driver("proc")->fstat_size(fd) : -1;
    default:
        return -1;
    }
}
