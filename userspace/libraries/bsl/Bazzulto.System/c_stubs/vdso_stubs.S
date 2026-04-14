// Bazzulto.System — vDSO C call stubs (AArch64 AAPCS64)
//
// Each stub computes the slot virtual address and branches to it.
// The SVC instruction lives in the kernel-managed vDSO page — never here.
//
// Slot VA = VDSO_BASE + slot * VDSO_SLOT_SIZE
//         = 0x1000   + slot * 16
//
// Arguments follow AAPCS64: x0–x5 in, x0 out. The vDSO stub ends with ret,
// so these trampolines are simple indirect branches — no stack frame needed.

.equ VDSO_BASE,      0x1000
.equ VDSO_SLOT_SIZE, 16

// Helper macro: branch to vDSO slot N.
// Uses x16 (intra-procedure-call scratch register — safe to clobber).
.macro VDSO_BRANCH slot
    mov  x16, #(VDSO_BASE + \slot * VDSO_SLOT_SIZE)
    br   x16
.endm

.text
.balign 16

// ---------------------------------------------------------------------------
// bz_exit(code: int32_t) -> noreturn
// ---------------------------------------------------------------------------
.global bz_exit
bz_exit:
    VDSO_BRANCH 0        // SLOT_EXIT

// ---------------------------------------------------------------------------
// bz_write(fd, buf, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_write
bz_write:
    VDSO_BRANCH 1        // SLOT_WRITE

// ---------------------------------------------------------------------------
// bz_read(fd, buf, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_read
bz_read:
    VDSO_BRANCH 2        // SLOT_READ

// ---------------------------------------------------------------------------
// bz_yield() -> int64_t
// ---------------------------------------------------------------------------
.global bz_yield
bz_yield:
    VDSO_BRANCH 3        // SLOT_YIELD

// ---------------------------------------------------------------------------
// bz_open(path, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_open
bz_open:
    VDSO_BRANCH 4        // SLOT_OPEN

// ---------------------------------------------------------------------------
// bz_close(fd) -> int64_t
// ---------------------------------------------------------------------------
.global bz_close
bz_close:
    VDSO_BRANCH 5        // SLOT_CLOSE

// ---------------------------------------------------------------------------
// bz_seek(fd, offset, whence) -> int64_t
// ---------------------------------------------------------------------------
.global bz_seek
bz_seek:
    VDSO_BRANCH 6        // SLOT_SEEK

// ---------------------------------------------------------------------------
// bz_spawn(path, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_spawn
bz_spawn:
    VDSO_BRANCH 7        // SLOT_SPAWN

// ---------------------------------------------------------------------------
// bz_list(buf, buf_len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_list
bz_list:
    VDSO_BRANCH 8        // SLOT_LIST

// ---------------------------------------------------------------------------
// bz_wait(pid, status_out) -> int64_t
// ---------------------------------------------------------------------------
.global bz_wait
bz_wait:
    VDSO_BRANCH 9        // SLOT_WAIT

// ---------------------------------------------------------------------------
// bz_pipe(fd_pair) -> int64_t
// ---------------------------------------------------------------------------
.global bz_pipe
bz_pipe:
    VDSO_BRANCH 10       // SLOT_PIPE

// ---------------------------------------------------------------------------
// bz_dup(source_fd) -> int64_t
// ---------------------------------------------------------------------------
.global bz_dup
bz_dup:
    VDSO_BRANCH 11       // SLOT_DUP

// ---------------------------------------------------------------------------
// bz_dup2(source_fd, dest_fd) -> int64_t
// ---------------------------------------------------------------------------
.global bz_dup2
bz_dup2:
    VDSO_BRANCH 12       // SLOT_DUP2

// ---------------------------------------------------------------------------
// bz_mmap(addr, length, prot, flags) -> int64_t
// ---------------------------------------------------------------------------
.global bz_mmap
bz_mmap:
    VDSO_BRANCH 13       // SLOT_MMAP

// ---------------------------------------------------------------------------
// bz_munmap(addr, length) -> int64_t
// ---------------------------------------------------------------------------
.global bz_munmap
bz_munmap:
    VDSO_BRANCH 14       // SLOT_MUNMAP

// ---------------------------------------------------------------------------
// bz_fork() -> int64_t
// ---------------------------------------------------------------------------
.global bz_fork
bz_fork:
    VDSO_BRANCH 15       // SLOT_FORK

// ---------------------------------------------------------------------------
// bz_exec(path, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_exec
bz_exec:
    VDSO_BRANCH 16       // SLOT_EXEC

// ---------------------------------------------------------------------------
// bz_getpid() -> int64_t
// ---------------------------------------------------------------------------
.global bz_getpid
bz_getpid:
    VDSO_BRANCH 17       // SLOT_GETPID

// ---------------------------------------------------------------------------
// bz_getppid() -> int64_t
// ---------------------------------------------------------------------------
.global bz_getppid
bz_getppid:
    VDSO_BRANCH 18       // SLOT_GETPPID

// ---------------------------------------------------------------------------
// bz_clock_gettime(clock_id, ts_out) -> int64_t
// ---------------------------------------------------------------------------
.global bz_clock_gettime
bz_clock_gettime:
    VDSO_BRANCH 19       // SLOT_CLOCK_GETTIME

// ---------------------------------------------------------------------------
// bz_nanosleep(req) -> int64_t
// ---------------------------------------------------------------------------
.global bz_nanosleep
bz_nanosleep:
    VDSO_BRANCH 20       // SLOT_NANOSLEEP

// ---------------------------------------------------------------------------
// bz_sigaction(signal_number, handler_va, old_handler_out) -> int64_t
// ---------------------------------------------------------------------------
.global bz_sigaction
bz_sigaction:
    VDSO_BRANCH 21       // SLOT_SIGACTION

// ---------------------------------------------------------------------------
// bz_kill(pid, signal_number) -> int64_t
// ---------------------------------------------------------------------------
.global bz_kill
bz_kill:
    VDSO_BRANCH 22       // SLOT_KILL

// SLOT_SIGRETURN (23) is not exposed as a C function — it is called by the
// kernel signal trampoline only.

// ---------------------------------------------------------------------------
// bz_creat(path, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_creat
bz_creat:
    VDSO_BRANCH 24       // SLOT_CREAT

// ---------------------------------------------------------------------------
// bz_unlink(path, len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_unlink
bz_unlink:
    VDSO_BRANCH 25       // SLOT_UNLINK

// ---------------------------------------------------------------------------
// bz_fstat(fd, stat_out) -> int64_t
// ---------------------------------------------------------------------------
.global bz_fstat
bz_fstat:
    VDSO_BRANCH 26       // SLOT_FSTAT

// ---------------------------------------------------------------------------
// bz_setfgpid(pid) -> int64_t
// ---------------------------------------------------------------------------
.global bz_setfgpid
bz_setfgpid:
    VDSO_BRANCH 27       // SLOT_SETFGPID

// ---------------------------------------------------------------------------
// bz_disk_info(buf, buf_len) -> int64_t
// ---------------------------------------------------------------------------
.global bz_disk_info
bz_disk_info:
    VDSO_BRANCH 28       // SLOT_DISK_INFO
