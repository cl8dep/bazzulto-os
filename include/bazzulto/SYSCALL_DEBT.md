# Syscall Table — Current State

## Implemented Syscalls

| # | Name              | Signature                                      | Status |
|---|-------------------|------------------------------------------------|--------|
| 0 | `exit`            | `exit(int status)` → noreturn                  | Done   |
| 1 | `write`           | `write(fd, buf, len)` → bytes written          | Done   |
| 2 | `read`            | `read(fd, buf, len)` → bytes read              | Done   |
| 3 | `yield`           | `yield()` → 0                                  | Done   |
| 4 | `open`            | `open(path)` → fd                              | Done   |
| 5 | `close`           | `close(fd)` → 0                                | Done   |
| 6 | `seek`            | `seek(fd, offset, whence)` → new offset        | Done   |
| 7 | `spawn`           | `spawn(path, argv)` → child PID               | Done   |
| 8 | `list`            | `list(idx, name_buf, len)` → file size         | Done   |
| 9 | `wait`            | `wait(pid)` → exit status; -1=any child        | Done   |
|10 | `pipe`            | `pipe(fds[2])` → 0                             | Done   |
|11 | `dup`             | `dup(oldfd)` → new fd                          | Done   |
|12 | `dup2`            | `dup2(oldfd, newfd)` → newfd                   | Done   |
|13 | `mmap`            | `mmap(length)` → user VA                       | Done   |
|14 | `munmap`          | `munmap(addr)` → 0                             | Done   |
|15 | `fork`            | `fork()` → child PID (parent), 0 (child)       | Done   |
|16 | `exec`            | `exec(path)` → -1 on fail, no return on success| Done   |

## Still Missing

| Category        | Syscalls                                   | Notes                                |
|-----------------|--------------------------------------------|--------------------------------------|
| VFS operations  | `stat`, `mkdir`, `unlink`, `rename`        | ramfs is read-only; needs write layer|
| Signals         | `sigaction`, `sigprocmask`, `sigreturn`    | Requires per-process trampoline page |
| Networking      | `socket`, `bind`, `connect`, …            | No network stack yet                 |
| I/O multiplexing| `poll`, `select`                           | Deferred                             |
| Time            | `clock_gettime`, `nanosleep`               | Architected timer exists, no syscall |
| Threads         | `clone`, `futex`                           | After SMP                            |

## Syscall ABI

- Numbers passed as SVC immediate (ISS[15:0] of ESR_EL1), extracted by `ESR_SVC_IMM`.
- Arguments in x0–x5 per AAPCS64.
- Return value in x0.
- User buffers validated against 48-bit VA limit (`USER_ADDR_LIMIT = 0x0001000000000000`).
