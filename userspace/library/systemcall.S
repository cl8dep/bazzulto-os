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

.global exit
exit:
    svc #0
    b .             // unreachable — kernel never returns from SYS_EXIT

.global write
write:
    svc #1
    ret

.global read
read:
    svc #2
    ret

.global yield
yield:
    svc #3
    ret

.global open
open:
    svc #4
    ret

.global close
close:
    svc #5
    ret

.global seek
seek:
    svc #6
    ret

.global spawn
spawn:
    svc #7
    ret

.global list
list:
    svc #8
    ret

.global wait
wait:
    svc #9
    ret

.global pipe
pipe:
    svc #10
    ret

.global dup
dup:
    svc #11
    ret

.global dup2
dup2:
    svc #12
    ret

.global mmap
mmap:
    svc #13
    ret

.global munmap
munmap:
    svc #14
    ret

.global fork
fork:
    svc #15
    ret

// exec(path): x0=path, x1 must be 0 (no argv — kernel uses {path}).
// Without the explicit zero, x1 contains whatever the caller had, which the
// kernel would misinterpret as a user argv pointer.
.global exec
exec:
    mov x1, #0
    svc #16
    ret

// execv(path, argv): x0=path, x1=argv (NULL-terminated string array).
.global execv
execv:
    svc #16
    ret

.global getpid
getpid:
    svc #17
    ret

.global getppid
getppid:
    svc #18
    ret

// clock_gettime(clock_id, tp): x0=clock_id, x1=tp.
.global clock_gettime
clock_gettime:
    svc #19
    ret

// nanosleep(req, rem): x0=req, x1=rem.
.global nanosleep
nanosleep:
    svc #20
    ret

// sigaction(signum, handler): x0=signum, x1=handler_va.
.global sigaction
sigaction:
    svc #21
    ret

// kill(pid, signum): x0=pid, x1=signum.
.global kill
kill:
    svc #22
    ret
