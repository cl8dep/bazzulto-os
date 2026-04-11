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

/// Create a kernel pipe and return two file descriptors.
/// x0 = pointer to int[2] (user): [0] = read fd, [1] = write fd.
/// Returns 0 on success, -1 on error.
#define SYSTEMCALL_PIPE     10

/// Duplicate a file descriptor to the lowest free slot >= 3.
/// x0 = oldfd.
/// Returns new fd >= 0, or -1 on error.
#define SYSTEMCALL_DUP      11

/// Duplicate a file descriptor to a specific slot.
/// x0 = oldfd, x1 = newfd.
/// Closes newfd first if it is already open.
/// Returns newfd on success, -1 on error.
#define SYSTEMCALL_DUP2     12

/// Allocate anonymous memory (readable + writable, zeroed).
/// x0 = length in bytes (rounded up to page boundary).
/// Returns user virtual address on success, (uint64_t)-1 on error.
#define SYSTEMCALL_MMAP     13

/// Release memory previously returned by SYSTEMCALL_MMAP.
/// x0 = address (must match a value previously returned by mmap).
/// Returns 0 on success, -1 if the address is not a known mmap allocation.
#define SYSTEMCALL_MUNMAP   14

/// Fork the calling process.
/// Creates a copy of the current process with a deep-copied address space.
/// In the parent: returns the child PID (> 0).
/// In the child:  returns 0.
/// Returns -1 on failure.
#define SYSTEMCALL_FORK     15

/// Replace the current process image with a new ELF binary from ramfs.
/// x0 = path (user string, max 256 bytes).
/// x1 = argv (user pointer to NULL-terminated string array, or NULL).
///      If NULL, the new process receives only argv[0] = path.
/// On success: does not return to the caller — execution restarts at the new
///             ELF entry point with a fresh stack.
/// On failure: returns -1.
#define SYSTEMCALL_EXEC     16

/// Return the PID index of the calling process.
/// No arguments. Returns PID >= 1.
#define SYSTEMCALL_GETPID   17

/// Return the PID index of the calling process's parent.
/// No arguments. Returns parent PID >= 0 (0 = no parent / orphan).
#define SYSTEMCALL_GETPPID  18

/// Read the current time from the specified clock.
/// x0 = clock_id (0 = CLOCK_REALTIME/monotonic, 1 = CLOCK_MONOTONIC).
/// x1 = pointer to struct timespec (user) to receive the result.
/// Returns 0 on success, -1 on error.
#define SYSTEMCALL_CLOCK_GETTIME 19

/// Sleep for at least the time given in *req.
/// x0 = pointer to const struct timespec (requested duration).
/// x1 = pointer to struct timespec (remaining time on interrupt, may be NULL).
/// Returns 0 on success, -1 on error.
#define SYSTEMCALL_NANOSLEEP 20

/// Register a signal handler for the given signal number.
/// x0 = signum (1–31).
/// x1 = handler VA (0 = SIG_DFL/default action, 1 = SIG_IGN/ignore).
/// Returns 0 on success, -1 on invalid signum.
#define SYSTEMCALL_SIGACTION 21

/// Send a signal to a process.
/// x0 = target PID index.
/// x1 = signal number (1–31).
/// Returns 0 on success, -1 if the target process is not found.
#define SYSTEMCALL_KILL      22

/// Restore the CPU state saved before a signal handler was invoked.
/// Called by the signal trampoline page after the user handler returns.
/// Pops the struct signal_frame from the user stack and eret's to the
/// original interrupted context. No explicit arguments.
#define SYSTEMCALL_SIGRETURN 23

/// Create or truncate a file for writing and return an fd.
/// x0 = path (user string, max 256 bytes).
/// Returns fd >= 0, or -1 on error or read-only scheme.
#define SYSTEMCALL_CREAT    24

/// Delete a file by path.
/// x0 = path (user string, max 256 bytes).
/// Returns 0 on success, -1 on error.
#define SYSTEMCALL_UNLINK   25

/// Return the size of an open file.
/// x0 = fd, x1 = pointer to struct vfs_stat (user).
/// Returns 0 on success, -1 on error.
#define SYSTEMCALL_FSTAT    26

/// Set the terminal foreground process PID.
/// x0 = pid (0 = no foreground process — shell is back in control).
/// Ctrl+C sends SIGINT to this process until it is cleared.
/// Returns 0.
#define SYSTEMCALL_SETFGPID 27

// Query disk filesystem info (capacity, free clusters, etc).
// x0 = 0 → returns total capacity in sectors
// x1 = 1 → returns free clusters
// x2 = 2 → returns total clusters
// x3 = 3 → returns bytes per cluster
// Returns 0 on success with struct disk_info filled via x1, -1 on error.
// Simple form: x0 = 0, x1 = pointer to struct disk_info, x2 = sizeof(struct disk_info)
#define SYSTEMCALL_DISK_INFO 28

// Structure returned by SYSTEMCALL_DISK_INFO.
struct disk_info {
    unsigned long long capacity_sectors;  // total disk capacity (512B sectors)
    unsigned long long free_clusters;     // free clusters available
    unsigned long long total_clusters;    // total data clusters
    unsigned long long bytes_per_cluster; // cluster size in bytes
    int                ready;             // 1 if filesystem initialized, 0 otherwise
};

#define NR_SYSTEMCALLS 29

// Deliver any pending signals to the current process before returning to EL0.
// Called at the end of systemcall_dispatch and exception_handler_irq_el0.
// Modifies frame->elr / frame->sp if a signal is delivered to user space.
void systemcall_deliver_pending_signals(struct exception_frame *frame);

// Dispatch a system call from the exception frame.
// Called from exception_handler_sync_el0 when EC = EC_SVC_AARCH64.
// Arguments are in frame->x0 through frame->x5.
// Return value is written to frame->x0 (restored by eret).
void systemcall_dispatch(struct exception_frame *frame);

// Register the init process (PID 1 equivalent).
// Must be called by kernel_main after launching the first user process so that
// orphaned children are reparented to it when their parent exits.
void systemcall_set_init_process(uint16_t pid_index);
