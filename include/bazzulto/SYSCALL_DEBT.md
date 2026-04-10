# Technical Debt: Minimal Syscall Table

**Priority:** Medium
**Files:** `syscall.h`, `kernel/syscall/`

## Current State

Bazzulto exposes only 4 syscalls:

| # | Name        | Equivalent (Linux) |
|---|-------------|---------------------|
| 0 | `SYS_EXIT`  | `exit`              |
| 1 | `SYS_WRITE` | `write`             |
| 2 | `SYS_READ`  | `read`              |
| 3 | `SYS_YIELD` | `sched_yield`       |

For reference, Linux exposes ~450 and macOS ~500.

## What's Missing

- **Process management** — `fork`, `exec`, `wait`, `kill`, `getpid`. Needed for multi-process support.
- **Memory management** — `mmap`, `munmap`, `brk`. Needed for user-space dynamic allocation.
- **File system** — `open`, `close`, `stat`, `lseek`, `mkdir`, `unlink`. Needed for persistent storage.
- **Networking** — `socket`, `bind`, `listen`, `accept`, `send`, `recv`.
- **Signals** — `sigaction`, `sigprocmask`, `sigreturn`. Needed for async event handling.
- **I/O multiplexing** — `poll`, `select`, or epoll-equivalent.
- **Time** — `clock_gettime`, `nanosleep`.
- **Threads** — `clone`, `futex`. Needed for user-space threading.

## Suggested Next Steps

1. `mmap` / `munmap` — enables user-space heap allocators.
2. `fork` / `exec` / `wait` — enables launching child processes.
3. `open` / `close` / `stat` — enables a VFS layer.

## Risks of Deferring

- Without `mmap`, user-space programs cannot allocate memory dynamically.
- Without process syscalls, the scheduler can only run statically loaded tasks.
- The flat numbering (`0..NR_SYSCALLS`) should be validated as a stable ABI before user-space depends on it.
