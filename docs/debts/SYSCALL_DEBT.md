# Syscall Debt — Pending and Partially Implemented

Last updated: 2026-04-14. Reflects kernel state after the musl ABI alignment pass.

Fully working syscalls are not listed here. This document only tracks what is
missing, broken, or partially implemented.

---

---

## New syscalls with stub or incomplete implementations

These were added in the musl ABI alignment pass but are not fully implemented.

| # | Name | Status | What is missing |
|---|------|--------|-----------------|
| 120 | `openat` | Partial | Only AT_FDCWD supported; other dirfd values return EBADF |
| 121 | `fstatat` | Partial | AT_EMPTY_PATH (fstat by fd) not implemented; only path-based |
| 122 | `unlinkat` | Partial | Only AT_FDCWD; other dirfd values ignored |
| 123 | `mkdirat` | Partial | Only AT_FDCWD; other dirfd values ignored |
| 128 | `fcntl` | Partial | F_DUPFD / F_DUPFD_CLOEXEC require `dup_at_or_above` which is stubbed as EBADF |
| 129 | `mprotect` | Stub → 0 | Page table attribute updates not implemented; always succeeds |
| 134 | `waitid` | Partial | WNOHANG, WCONTINUED not respected; infop siginfo_t is minimal |
| 137 | `mremap` | ENOSYS | Requires VM remap API that does not exist yet |
| 140 | `symlink` | Implemented | Only works on tmpfs (FAT32 rejects link_child) |
| 141 | `link` | ENOSYS | Hard links require link-count tracking not yet in VFS |
| 145 | `fchdir` | Partial | cwd_path reconstruction only works for mount-root inodes |
| 146 | `statx` | Partial | No timestamps (atime/mtime/ctime), no block device info, no uid/gid per inode |
| 151 | `renameat` | Partial | Only AT_FDCWD; other dirfd values return EBADF |
| 156–159 | `timer_create/settime/gettime/delete` | ENOSYS | Requires per-process timer queue subsystem |
| 160 | `setitimer` | Partial | ITIMER_REAL only; no periodic re-arming (single-shot like alarm); ITIMER_VIRTUAL/PROF → ENOSYS |
| 161 | `getitimer` | Partial | ITIMER_REAL only |

---

## VFS and permission layer debt

| Area | What is missing |
|------|-----------------|
| `chmod`/`fchmod` (108, 109) | Succeed silently; inode mode bits not enforced at access time |
| `chown`/`fchown`/`lchown` (110–112) | Succeed silently; no per-inode uid/gid storage in InodeStat |
| `fchmodat`/`fchownat` (144, 143) | Same as above |
| `statx` (146) | atime/mtime/ctime always zero; no per-inode timestamp tracking |
| `link` (141) | FAT32 and tmpfs have no hard link count tracking |
| FAT32 symlinks | `link_child` returns NotSupported on Fat32DirInode; symlinks only work on tmpfs |
| Dirfd resolution | All `*at` syscalls with a real dirfd (not AT_FDCWD) return EBADF; requires reverse path table or open-fd path storage |

---

## Signal subsystem debt

| Area | What is missing |
|------|-----------------|
| Signal mask enforcement | `sa_mask` in sigaction is accepted but not applied to block signals during handler execution |

---

## Threading debt

| Area | What is missing |
|------|-----------------|
| `clone` ptid write | CLONE_PARENT_SETTID now writes child TID to `*ptid`; CLONE_CHILD_CLEARTID now stores ctid in `clear_child_tid` |
| Robust futex list | `set_robust_list` stores the head but the kernel does not yet walk it on thread exit to unlock held futexes |
| `clear_child_tid` zeroing | `clear_child_tid` is stored but the scheduler does not yet zero it and futex-wake on thread exit |
| `tgkill` | TGID is not validated; treated the same as tkill |

---

