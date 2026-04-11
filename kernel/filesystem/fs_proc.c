// //proc:<pid>/ scheme driver — process information as read-only virtual files.
//
// Authority = decimal PID string, e.g. "42".
// Supported virtual paths:
//   /status  — formatted text snapshot: pid, ppid, state, name
//
// Snapshots are pre-built into a proc_snapshot_t at open() time.
// There is no live updating — reads return the state at open time.

#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/virtual_file_system.h"
#include "../../include/bazzulto/scheduler.h"
#include "../../include/bazzulto/heap.h"
#include "errno.h"
#include <string.h>
#include <stddef.h>

// ---------------------------------------------------------------------------
// Simple number → string helper (no libc dependency in kernel)
// ---------------------------------------------------------------------------

static int uint_to_decimal(char *out, size_t out_size, uint64_t value)
{
    if (out_size == 0) return 0;
    char tmp[24];
    int  length = 0;
    if (value == 0) {
        tmp[length++] = '0';
    } else {
        while (value > 0 && length < (int)sizeof(tmp) - 1) {
            tmp[length++] = (char)('0' + (value % 10));
            value /= 10;
        }
    }
    // reverse
    int written = 0;
    for (int i = length - 1; i >= 0 && (size_t)written < out_size - 1; i--)
        out[written++] = tmp[i];
    out[written] = '\0';
    return written;
}

static size_t append_str(char *buf, size_t pos, size_t max, const char *str)
{
    while (*str && pos < max - 1)
        buf[pos++] = *str++;
    buf[pos] = '\0';
    return pos;
}

static size_t append_u64(char *buf, size_t pos, size_t max, uint64_t value)
{
    char num[24];
    uint_to_decimal(num, sizeof(num), value);
    return append_str(buf, pos, max, num);
}

static const char *state_name(process_state_t state)
{
    switch (state) {
    case PROCESS_STATE_READY:   return "ready";
    case PROCESS_STATE_RUNNING: return "running";
    case PROCESS_STATE_BLOCKED: return "blocked";
    case PROCESS_STATE_WAITING: return "waiting";
    case PROCESS_STATE_ZOMBIE:  return "zombie";
    case PROCESS_STATE_DEAD:    return "dead";
    default:                    return "unknown";
    }
}

// ---------------------------------------------------------------------------
// Scheme open
// ---------------------------------------------------------------------------

int fs_proc_open(const char *authority, const char *path,
                 file_descriptor_t *fd_out)
{
    // authority = decimal PID string.
    uint16_t pid_index = 0;
    const char *cursor = authority;
    while (*cursor >= '0' && *cursor <= '9')
        pid_index = (uint16_t)(pid_index * 10 + (*cursor++ - '0'));

    if (pid_index == 0)
        return -ENOENT;

    process_t *process = scheduler_find_process(pid_index);
    if (!process)
        return -ENOENT;

    if (strcmp(path, "/status") != 0)
        return -ENOENT;

    // Build snapshot.
    proc_snapshot_t *snapshot = (proc_snapshot_t *)kmalloc(sizeof(proc_snapshot_t));
    if (!snapshot)
        return -ENOMEM;

    char *buf   = snapshot->buf;
    size_t max  = PROC_SNAPSHOT_SIZE;
    size_t pos  = 0;

    pos = append_str(buf, pos, max, "pid: ");
    pos = append_u64(buf, pos, max, (uint64_t)process->pid.index);
    pos = append_str(buf, pos, max, "\nppid: ");
    pos = append_u64(buf, pos, max, (uint64_t)process->parent_pid);
    pos = append_str(buf, pos, max, "\nstate: ");
    pos = append_str(buf, pos, max, state_name(process->state));
    pos = append_str(buf, pos, max, "\nname: ");
    pos = append_str(buf, pos, max, process->name[0] ? process->name : "(unknown)");
    pos = append_str(buf, pos, max, "\n");

    snapshot->size = pos;

    fd_out->type   = FD_TYPE_PROC;
    fd_out->proc   = snapshot;
    fd_out->offset = 0;
    return 0;
}

int fs_proc_creat(const char *authority __attribute__((unused)),
                  const char *path __attribute__((unused)),
                  file_descriptor_t *fd_out __attribute__((unused)))
{
    return -EROFS;
}

int fs_proc_unlink(const char *authority __attribute__((unused)),
                   const char *path __attribute__((unused)))
{
    return -EROFS;
}

int64_t fs_proc_fstat_size(const file_descriptor_t *fd)
{
    if (fd->type != FD_TYPE_PROC || !fd->proc)
        return -EBADF;
    return (int64_t)fd->proc->size;
}

const vfs_scheme_driver_t fs_proc_driver = {
    .scheme     = "proc",
    .open       = fs_proc_open,
    .creat      = fs_proc_creat,
    .unlink     = fs_proc_unlink,
    .fstat_size = fs_proc_fstat_size,
};
