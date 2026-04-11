# Syscall Table — Current State

This file tracks the kernel syscall ABI in `include/bazzulto/systemcall.h`.
The numeric ABI is frozen for the currently implemented syscalls `0..28`.

## Implemented Syscalls

| # | Name | Kernel status |
|---|------|---------------|
| 0 | `exit` | Done |
| 1 | `write` | Done |
| 2 | `read` | Done |
| 3 | `yield` | Done |
| 4 | `open(path)` | Done |
| 5 | `close` | Done |
| 6 | `seek` | Done |
| 7 | `spawn` | Done |
| 8 | `list` | Done |
| 9 | `wait(pid)` | Done |
| 10 | `pipe` | Done |
| 11 | `dup` | Done |
| 12 | `dup2` | Done |
| 13 | `mmap` | Done |
| 14 | `munmap` | Done |
| 15 | `fork` | Done |
| 16 | `exec(path, argv)` | Done |
| 17 | `getpid` | Done |
| 18 | `getppid` | Done |
| 19 | `clock_gettime` | Done |
| 20 | `nanosleep` | Done |
| 21 | `sigaction` | Done |
| 22 | `kill` | Done |
| 23 | `sigreturn` | Done |
| 24 | `creat` | Done |
| 25 | `unlink` | Done |
| 26 | `fstat` | Done |
| 27 | `setfgpid` | Done |
| 28 | `disk_info` | Done |

## Current POSIX-first debt

| Area | Missing / incomplete |
|------|----------------------|
| Error model | Kernel still needs broader `-errno` coverage outside the first POSIX foundation pass |
| VFS | `stat(path)`, `lstat`, `mkdir`, `rmdir`, `rename`, directory enumeration, relative paths, `cwd` |
| Public libc | `open(path, flags, ...)` semantics are only partially emulated in libc for now |
| Process API | `waitpid`, `execve`, `fcntl(FD_CLOEXEC)` not implemented end-to-end yet |
| Signals | `sigaction` is still a minimal handler registration API; no `sigprocmask` yet |
| Time | `CLOCK_REALTIME` vs `CLOCK_MONOTONIC` still need POSIX-correct separation |

## ABI notes

- Syscall numbers are passed in the SVC immediate (`ISS[15:0]` of `ESR_EL1`).
- Arguments use `x0`–`x5`; the raw return value is written to `x0`.
- The raw userspace syscall layer now uses `sys_*` names in `userspace/library/systemcall.*`.
- Public applications should prefer `userspace/libc/*`; libc translates negative kernel returns into `errno` + `-1`.
