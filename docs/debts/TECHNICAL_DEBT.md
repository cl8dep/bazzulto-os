# Technical Debt

## 1. Limits & Tunables — RESOLVED

### 1.1 Kernel Stack per Process — DONE
`KERNEL_STACK_SIZE` increased from 16 KB → 24 KB in `kernel/scheduler/scheduler.c`.

### 1.2 Max Processes / PID Max — DONE
PID recycling is fully implemented in `kernel/scheduler/pid.c` using a bitmap with
`pid_next_hint` wrap-around and generation counters. `scheduler_reap_process` calls
`pid_free()` which clears the bitmap bit.

### 1.3 File Descriptor Limit per Process — DONE
`VIRTUAL_FILE_SYSTEM_MAX_FDS` increased from 16 → 64 in
`include/bazzulto/virtual_file_system.h`.

### 1.4 Max Files in ramfs — DONE
`RAMFS_MAX_FILES` increased from 32 → 128, `RAMFS_MAX_NAME` from 64 → 255 in
`include/bazzulto/ramfs.h`.

---

## 2. Memory — MOSTLY RESOLVED

### 2.1 Heap Abstraction — DONE
`heap_grow` now calls `kernel_vm_alloc(vaddr, 1, flags)` instead of calling
`physical_memory_alloc` + `virtual_memory_map` directly.
`kernel_vm_alloc` is defined in `kernel/memory/virtual_memory.c`.

### 2.2 Heap Double-Free Detection — DONE
`kfree` panics if `block->is_free == 1` at entry (double-free detected).

### 2.3 User Stack Guard Page — DONE
The page at `stack_top - (USER_STACK_PAGES + 1) * PAGE_SIZE` is intentionally
unmapped. Stack overflow faults there before touching any other mapping.
Documented in `kernel/loader/elf_loader.c` with `USER_STACK_GUARD_PAGES` macro.

### 2.4 Stack ASLR — DONE
`aslr_stack_offset()` in `elf_loader.c` reads `CNTPCT_EL0` (physical timer),
mixes it with the page table pointer, and shifts the stack base by 0–255 pages.
Full text-segment ASLR requires PIE compilation (`-fPIE`); not implemented since
userspace programs use `-fno-pic`.

### 2.5 Anonymous mmap / munmap — DONE
`sys_mmap` / `sys_munmap` (syscalls 13/14) allocate and free anonymous zeroed pages
in a per-process bump-pointer region starting at `MMAP_USER_BASE = 0x200000000`.
Up to `PROCESS_MMAP_MAX_REGIONS = 16` regions per process tracked in `process_t`.
Userspace wrappers: `mmap(size)` / `munmap(addr)` in `userspace/library/systemcall.h`.

### 2.6 Heap Size — OPEN
`HEAP_MAX` is still fixed at 64 MB from `0xFFFFFFFF90000000`. Dynamic growth
up to a fraction of physical RAM is not yet implemented.

### 2.7 PMM: Buddy System — OPEN
Physical memory allocator is still a free list. Buddy system needed to prevent
fragmentation under heavy load.

---

## 3. Security — OPEN

### 3.1 Syscall Caller Verification — OPEN
Not yet implemented. All userspace `svc #N` calls are dispatched unconditionally.
Relevant once untrusted third-party code runs on Bazzulto.

---

## 4. Syscall Surface — MOSTLY RESOLVED

See `include/bazzulto/SYSCALL_DEBT.md` for the up-to-date syscall table.

### Resolved
- `pipe` / `dup` / `dup2` — IPC, I/O redirection (syscalls 10–12)
- `mmap` / `munmap` — user-space dynamic memory (syscalls 13–14)
- `fork` — deep-copy process duplication (syscall 15)
- `exec` — replace process image with new ELF from ramfs (syscall 16)
- `getpid` / `getppid` — process identifiers (syscalls 17–18)
- `clock_gettime` / `nanosleep` — basic time syscalls exist (syscalls 19–20)
- `sigaction` / `kill` / `sigreturn` — minimal signal delivery exists (syscalls 21–23)
- `creat` / `unlink` / `fstat` — basic file mutation / metadata hooks exist (syscalls 24–26)

### Still Open
- Full POSIX VFS surface — `stat(path)`, `lstat`, `mkdir`, `rmdir`, `rename`,
  directory enumeration, relative paths, and per-process `cwd`
- libc/public API cleanup — keep raw `sys_*` wrappers internal and expose POSIX
  semantics from `userspace/libc/*`
- Broader `-errno` coverage — the first POSIX foundation pass is in place, but
  more filesystem and process paths still need specific error returns
- Signals — `sigaction` is still minimal; `sigprocmask`, `SA_RESTART`, and more
  POSIX-correct interruption semantics remain open
- `socket` / networking — not on the near-term roadmap
- `poll` / `select` — I/O multiplexing; deferred
- `clock_gettime` realtime split — `CLOCK_REALTIME` still needs RTC-backed wall clock
- `clone` / `futex` — user-space threading; deferred

---

## 5. Scheduler — OPEN

### 5.1 Round Robin → MLFQ — OPEN
Scheduler is still round-robin. MLFQ (multi-level feedback queue) is the target
for interactive responsiveness but is not yet implemented.

### 5.2 SMP — OPEN
Single-core only. Multi-core support requires per-core run queues and spinlocks.
