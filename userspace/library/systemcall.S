// User-space system call wrappers for Bazzulto OS.
//
// Each wrapper issues `svc #N` where N is the syscall number.
// The SVC immediate is extracted by the kernel from ESR_EL1 ISS[15:0]
// (ARM ARM D13.2.36).
//
// Arguments are passed in x0-x5 per AAPCS64, which matches the kernel's
// syscall convention. Return value is in x0.
//
// Syscall numbers (must match include/bazzulto/systemcall.h):
//   SYSTEMCALL_EXIT  = 0
//   SYSTEMCALL_WRITE = 1
//   SYSTEMCALL_READ  = 2
//   SYSTEMCALL_YIELD = 3
//   SYSTEMCALL_OPEN  = 4
//   SYSTEMCALL_CLOSE = 5
//   SYSTEMCALL_SEEK  = 6
//   SYSTEMCALL_SPAWN = 7
//   SYSTEMCALL_LIST  = 8
//   SYSTEMCALL_WAIT  = 9
//   SYSTEMCALL_PIPE  = 10
//   SYSTEMCALL_DUP   = 11
//   SYSTEMCALL_DUP2  = 12
//   SYSTEMCALL_MMAP  = 13
//   SYSTEMCALL_MUNMAP= 14
//   SYSTEMCALL_FORK  = 15
//   SYSTEMCALL_EXEC  = 16

.text

.global sys_exit
sys_exit:
    svc #0
    b .             // unreachable — kernel never returns from SYS_EXIT

.global sys_write
sys_write:
    svc #1
    ret

.global sys_read
sys_read:
    svc #2
    ret

.global sys_yield
sys_yield:
    svc #3
    ret

.global sys_open
sys_open:
    svc #4
    ret

.global sys_close
sys_close:
    svc #5
    ret

.global sys_seek
sys_seek:
    svc #6
    ret

.global sys_spawn
sys_spawn:
    svc #7
    ret

.global sys_list
sys_list:
    svc #8
    ret

.global sys_wait
sys_wait:
    svc #9
    ret

.global sys_pipe
sys_pipe:
    svc #10
    ret

.global sys_dup
sys_dup:
    svc #11
    ret

.global sys_dup2
sys_dup2:
    svc #12
    ret

.global sys_mmap
sys_mmap:
    svc #13
    ret

.global sys_munmap
sys_munmap:
    svc #14
    ret

.global sys_fork
sys_fork:
    svc #15
    ret

// exec(path): x0=path, x1 must be 0 (no argv — kernel uses {path}).
// Without the explicit zero, x1 contains whatever the caller had, which the
// kernel would misinterpret as a user argv pointer.
.global sys_exec
sys_exec:
    mov x1, #0
    svc #16
    ret

// execv(path, argv): x0=path, x1=argv (NULL-terminated string array).
.global sys_execv
sys_execv:
    svc #16
    ret

.global sys_getpid
sys_getpid:
    svc #17
    ret

.global sys_getppid
sys_getppid:
    svc #18
    ret

// clock_gettime(clock_id, tp): x0=clock_id, x1=tp.
.global sys_clock_gettime
sys_clock_gettime:
    svc #19
    ret

// nanosleep(req, rem): x0=req, x1=rem.
.global sys_nanosleep
sys_nanosleep:
    svc #20
    ret

// sigaction(signum, handler): x0=signum, x1=handler_va.
.global sys_sigaction
sys_sigaction:
    svc #21
    ret

// kill(pid, signum): x0=pid, x1=signum.
.global sys_kill
sys_kill:
    svc #22
    ret

// creat(path): x0=path.
.global sys_creat
sys_creat:
    svc #24
    ret

// unlink(path): x0=path.
.global sys_unlink
sys_unlink:
    svc #25
    ret

// fstat(fd, stat): x0=fd, x1=stat.
.global sys_fstat
sys_fstat:
    svc #26
    ret

// set_terminal_foreground_pid(pid): x0=pid.
.global sys_set_terminal_foreground_pid
sys_set_terminal_foreground_pid:
    svc #27
    ret

// disk_info(info): x0=pointer to struct disk_info.
// Returns 0 on success, -1 on error.
.global sys_disk_info
sys_disk_info:
    svc #28
    ret
