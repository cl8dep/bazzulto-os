# Kernel Debt — Index

This directory documents known gaps, pending work, and technical debt in the
Bazzulto kernel. Each file covers one subsystem area.

---

## Files

| File | Subsystem | Priority |
|---|---|---|
| [NETWORK_DEBT.md](NETWORK_DEBT.md) | TCP/IP stack, VirtIO-net, BSD sockets for AF_INET | High |
| [SECURITY_DEBT.md](SECURITY_DEBT.md) | UID/GID, file permissions, copy_from_user validation | High |
| [MEMORY_DEBT.md](MEMORY_DEBT.md) | Demand paging, buddy allocator, MAP_SHARED in fork, ASLR entropy | High |
| [KERNEL_MISC_DEBT.md](KERNEL_MISC_DEBT.md) | select(), SCM_RIGHTS, SIGALRM, FD ref-count for threads, /proc/self, PT_INTERP, RTC | Medium |
| [TECHNICAL_DEBT.md](TECHNICAL_DEBT.md) | Legacy C kernel debt (mostly resolved) | Low |
| [SYSCALL_DEBT.md](SYSCALL_DEBT.md) | Syscall surface tracking (legacy C kernel) | Historical |
| [KEYBOARD_DEBT.md](KEYBOARD_DEBT.md) | Keyboard driver debt (legacy C kernel) | Historical |

---

## Summary of open items by severity

### Must close before multi-user / production use

- `copy_from_user` / `copy_to_user` range validation across all syscall paths
- UID/GID per-process model and file permission enforcement
- `kill()` permission check

### Must close for real workloads

- Networking: VirtIO-net → Ethernet → IPv4 → TCP/UDP → BSD socket AF_INET
- Demand paging (lazy BSS / file-backed pages)
- FD table shared between threads (`Arc<SpinLock<FdTable>>`)

### Should close for POSIX completeness

- `select()` syscall
- `alarm()` / `SIGALRM`
- `/proc/self` symlink + symlink resolution in VFS
- `CLOCK_REALTIME` backed by PL031 RTC
- `SCM_RIGHTS` (fd passing over Unix sockets)
- `nlinks` discriminant collision in IPC inodes

### Correctness refinements

- `MAP_SHARED` regions excluded from CoW in `fork()`
- Buddy allocator replacing free-list PMM
- Kernel stack guard pages
- ASLR entropy pool

### Future

- ELF `PT_INTERP` dynamic linker support
- `seccomp` syscall filtering
- Swap / page eviction
