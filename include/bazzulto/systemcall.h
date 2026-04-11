#pragma once

#include <stdint.h>
#include "exceptions.h"

// System call numbers — passed in the SVC immediate (ISS[15:0] of ESR_EL1).
// User code invokes: svc #NR
//
// All arguments are passed via registers x0–x5.
// Return value is written to x0 (restored by eret).
// User buffers are validated against the TTBR0 48-bit VA limit.
//
// Full documentation: https://github.com/cl8dep/bazzulto-os/wiki/System-Calls

/// Terminate the calling process.
/// x0 = exit status (currently unused).
/// Marks the process as DEAD and yields. Never returns.
#define SYSTEMCALL_EXIT     0

/// Write bytes to an open file descriptor.
/// x0 = fd, x1 = buf (user pointer), x2 = len.
/// Returns bytes written, or -1 on error.
#define SYSTEMCALL_WRITE    1

/// Read bytes from an open file descriptor.
/// x0 = fd, x1 = buf (user pointer), x2 = len.
/// Returns bytes read, or -1 on error. Returns 0 if len is 0.
#define SYSTEMCALL_READ     2

/// Voluntarily give up the CPU to the scheduler.
/// No arguments. Returns 0.
#define SYSTEMCALL_YIELD    3

/// Open a file by path and allocate a file descriptor.
/// x0 = path (user string, max 256 bytes).
/// Returns fd >= 0, or -1 if not found / no free fd.
#define SYSTEMCALL_OPEN     4

/// Close an open file descriptor.
/// x0 = fd.
/// Returns 0 on success, -1 on bad fd.
#define SYSTEMCALL_CLOSE    5

/// Reposition the read/write offset of a file descriptor.
/// x0 = fd, x1 = offset, x2 = whence (SEEK_SET, SEEK_CUR, SEEK_END).
/// Returns new offset, or -1 on error.
#define SYSTEMCALL_SEEK     6

/// Load an ELF binary from ramfs and create a new process.
/// x0 = path (user string, max 256 bytes).
/// Returns child PID >= 0, or -1 on failure.
#define SYSTEMCALL_SPAWN    7

/// List a file in the ramfs by index.
/// x0 = index (0-based), x1 = name_buf (user pointer), x2 = buf_len.
/// Copies the file name into name_buf (null-terminated).
/// Returns the file size in bytes, or -1 if index is out of range.
#define SYSTEMCALL_LIST     8

/// Block until the process with the given PID exits.
/// x0 = pid (must be a valid child PID returned by SYSTEMCALL_SPAWN).
/// Returns 0 when the child has exited, or -1 if the PID is not found.
#define SYSTEMCALL_WAIT     9

#define NR_SYSTEMCALLS 10

// Dispatch a system call from the exception frame.
// Called from exception_handler_sync_el0 when EC = EC_SVC_AARCH64.
// Arguments are in frame->x0 through frame->x5.
// Return value is written to frame->x0 (restored by eret).
void systemcall_dispatch(struct exception_frame *frame);
