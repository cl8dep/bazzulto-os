# Bazzulto OS — Roadmap to v1.0

> **This document is authoritative.** Every task listed here must be completed before `v1.0` is tagged.
> Anything not listed here is explicitly deferred to post-v1.0.

## Design Philosophy

**El BPM reemplaza a sudo/su como mecanismo de privilegio.** En el modelo Unix clásico, la escalada de privilegios pasa por cambiar de identidad (uid=0). En Bazzulto, la escalada pasa por que un binario tenga las capabilities correctas para el namespace que necesita acceder. Una herramienta que necesita montar un filesystem tiene `//sys:mount/**`; no necesita convertirse en root.

Consecuencias de este modelo:
- El usuario uid=0 se llama **`system`**, no `root`. Es la identidad bajo la que corren los servicios del sistema, no un superusuario interactivo.
- **`su` y `sudo` no son el flujo normal.** Existen por compatibilidad POSIX pero se desincentivan. El administrador no hace `su system && mount /dev/sdb /mnt`; usa `mount` (Tier 1, tiene `//sys:mount/**`) directamente.
- **DAC y BPM son ortogonales.** `su system` cambia el eje DAC (uid=0 bypasea chmod/chown). El eje BPM no cambia: cada binario sigue evaluándose por su hash. Un binario desconocido ejecutado desde una shell de `system` sigue recibiendo el prompt o la denegación.
- **Root bypass en DAC, no en BPM.** `euid=0` bypasea los checks de `vfs_check_access()` (DAC). Nunca bypasea `permission_allows()` (BPM).

---

## State on Entry (v0.1-stable)

**Complete and stable:**
- AArch64 boot: UEFI → edk2 → Limine → `kernel_main()`
- AArch64 exception handling: full 16-entry vector table, EL0/EL1, frame decode
- Memory: free-list physical allocator, 4-level ARMv8 page tables, slab heap
- Scheduler: 40-level round-robin, fork/exec/wait/exit, CoW, stack ASLR + guard page
- Signals: `sigaction`, `kill`, `sigreturn`, `sigprocmask` (SIGALRM broken, see M2)
- VFS: FAT32 (production-ready), BAFS v0.1 (implemented, **not default**), devfs, procfs, ramfs, tmpfs, FIFO
- Syscall surface: 162 entries, ~80% fully implemented
- IPC: AF_UNIX sockets (stream + dgram), named/anonymous pipes, POSIX semaphores, POSIX message queues, epoll
- ELF loader: static ELF64 only; no `PT_INTERP`
- vDSO: syscall stubs at `0x7fff_0000`
- Display: Limine framebuffer + JetBrainsMono text rendering via `bzdisplayd`
- bzinit: dependency-ordered service manager
- Userspace: `bzsh` (basic), 34 coreutils, BSL (System/IO/Display/Diagnostics)
- Binary Permission Model: Tier 1 path-based capability checks only

**Currently incomplete / stubbed:**
- `chmod`, `fchmod`, `chown`, `fchown` → stubs
- `select()` → `ENOSYS`
- `SCM_RIGHTS` → not implemented
- `SIGALRM` / `alarm()` → not delivered
- `FdTable` thread sharing → threads receive private copies instead of shared reference
- `/proc/self` symlink → broken
- `PT_INTERP` / dynamic linker → static ELF only
- `MAP_SHARED` → silently treated as `MAP_PRIVATE`
- `copy_from_user` / `copy_to_user` bounds checking → overflow-only
- UID/GID per process → always 0
- File permission bits → stored, never enforced
- `kill()` permission check → sends to any PID unconditionally
- `CLOCK_REALTIME` → returns monotonic ticks, not wall clock
- Kernel stack guard pages → absent
- ASLR entropy → weak (CNTPCT_EL0 only)
- BAFS not the root filesystem → FAT32 is (v1.0 will use Btrfs as root while BAFS stabilizes)
- Binary Permission Model Tiers 2–4 → not implemented
- Shell: missing `test`/`[`, `printf` builtin, `(( ))`, `local`, globbing, job control, history, line editing
- musl libc → patch overlay exists, not fully tested
- Dynamic linker → absent
- Text editor → absent
- Boot splash → absent
- Full font management → partial
- bzinit service set → incomplete

---

## What v1.0 Explicitly Excludes

These items are not in scope. They must not appear in any v1.0 task or block any v1.0 milestone.

- Network stack (TCP/IP, UDP, VirtIO-net, AF_INET sockets)
- SMP / multi-core CPU startup
- GUI, windowing system, or desktop environment
- Package manager (`baz install`)
- Compiler or toolchain running on the OS itself
- seccomp / BPF syscall filtering
- Binary Permission Model Tier 3 — ELF `.bazzulto_permissions` section parsing and declarative per-permission approval dialog (showing the user exactly which namespaces a binary is requesting)
- Binary Permission Model — Ed25519 runtime signature verification of binaries
- Binary Permission Model — `sys_request_cap()`, Powerbox, `sys_restrict_self()`
- `awk` interpreter
- Bazzulto.Crypto, Bazzulto.Net, Bazzulto.Storage (BSL modules)
- Buddy physical memory allocator (optimization, not correctness)
- Swap / paging to disk
- Audio, USB, Bluetooth subsystems
- App Permission Model (`docs/features/App Permission Model.md`)
- VirtualBox installer (live-boot from ISO is sufficient)

---

## Milestone Map

| # | Milestone | Version Tag |
|---|-----------|-------------|
| M1 | Syscall ABI Freeze | v0.2 |
| M2 | Kernel Correctness — Critical Fixes | v0.3 |
| M3 | POSIX Identity and DAC | v0.4 |
| M4 | Btrfs as Default Root Filesystem | v0.5 |
| M5 | Binary Permission Model — Tier 2 | v0.5.1 |
| M6 | musl libc Integration and Test | v0.6 |
| M7 | Dynamic Linker (bzdld) | v0.6.1 |
| M8 | Terminal Experience and Shell Completeness | v0.7 |
| M9 | Coreutils Completeness | v0.7.1 |
| M10 | Ported Text Editor — nano | v0.8 |
| M11 | bzinit — Full Service Definitions | v0.8.1 |
| M12 | Boot Splash and Font System | v0.9 |
| M13 | BSL 1.0 API Freeze | v0.9.1 |
| M14 | Integration Testing — v1.0-rc | v1.0-rc |
| M15 | ISO Packaging and v1.0 Release | v1.0 |

**Dependency order:**
```
M1 → M2 → M3 → M4 → M5
               M3 → M9
M1 → M6 → M7 → M10
M1 → M8 → M9 → M10
M5, M9, M10 → M11 → M12
M7, M12, M13 → M14 → M15
```

M6 (musl) may start in parallel with M3. M8 (terminal/shell) may start in parallel with M6.

---

## M1 — Syscall ABI Freeze (→ v0.2)

**Goal:** Freeze syscall numbering forever. After this tag, no syscall number may change. New syscalls are added at the next available slot ≥ 162. This tag is a hard prerequisite for musl and for any C program that compiles syscall numbers at build time.

### Tasks

**1.1** Update `docs/wiki/System-Calls.md` (wiki submodule). List all 162 syscalls (numbers 0–161) with: syscall number, name, C-equivalent signature (argument types + return type), errno values, and one stability marker:
- `[STABLE]` — fully implemented, behavior frozen
- `[PROVISIONAL]` — implemented but behavior may change; number is frozen
- `[RESERVED]` — currently returns `ENOSYS`; number reserved for future use

Assign `[STABLE]` to: `exit`, `write`, `read`, `seek`, `open`, `close`, `yield`, `spawn`, `list`, `wait`, `pipe`, `dup`, `dup2`, `mmap`, `munmap`, `fork`, `exec`, `getpid`, `getppid`, `clock_gettime`, `nanosleep`, `sigaction`, `kill`, `sigreturn`, `sigprocmask`, `creat`, `unlink`, `fstat`, `disk_info`, `mkdir`, `chdir`, `getcwd`, `getdents64`, `ioctl`, `tcgetattr`, `tcsetattr`, `truncate`, `fsync`, `readlink`, `symlink`, `rename`, `poll`, `epoll_create1`, `epoll_ctl`, `epoll_wait`, `clone`, `set_tls`, `gettid`, `futex`, `mount`, `getmounts`, `sem_open`, `sem_close`, `sem_wait`, `sem_timedwait`, `sem_post`, `sem_getvalue`, `mq_open`, `mq_close`, `mq_send`, `mq_receive`, `mq_getattr`, `socket`, `bind`, `listen`, `accept`, `connect`, `send`, `recv`, `framebuffer_map`, `uname`, `sysinfo`, `prctl`, `pread64`, `pwrite64`, `readv`, `writev`, `fcntl`, `openat`, `set_tid_address`, `exit_group`, `brk`, `tkill`, `tgkill`, `mprotect`.

Assign `[PROVISIONAL]` to: `chmod`, `fchmod`, `chown`, `fchown`, `select`, `getuid`, `setuid`, `getgid`, `setgid`, `geteuid`, `getegid`, `setreuid`, `setregid`, `getgroups`, `setgroups`, `umask`, `alarm`, `sendmsg`, `recvmsg`, `timer_create`, `timer_settime`, `timer_gettime`, `timer_delete`.

Assign `[RESERVED]` to any remaining entries that currently return `ENOSYS` and are not targeted for v1.0.

**1.2** In `kernel/src/systemcalls/mod.rs`, add a top-of-file comment block stating:

```rust
// SYSCALL ABI — FROZEN AT v0.2
// Syscall numbers 0–161 are immutable.
// To add a new syscall, use the next available number >= 162.
// Never reassign an existing number.
```

**1.3** In `kernel/src/vdso/mod.rs`, add compile-time assertions (`abi_frozen!` macro) that verify every syscall number constant matches its documented ABI value. If any constant is renumbered, the kernel fails to compile. This replaces the need for a runtime vDSO test since the vDSO generates `svc #N` for slot N at compile time.

**1.4** Tag commit `v0.2-syscall-abi-freeze`.

### Exit criteria
`docs/wiki/System-Calls.md` exists and is complete. Kernel compiles with all 162 `abi_frozen!` assertions passing. Any syscall renumbering produces a compile error.

---

## M2 — Kernel Correctness — Critical Fixes (→ v0.3)

**Goal:** Resolve every item in `docs/debts/SECURITY_DEBT.md`, `docs/debts/MEMORY_DEBT.md`, and `docs/debts/KERNEL_MISC_DEBT.md` that is in scope for v1.0.

### Already complete (verified in codebase)

The following were listed as M2 tasks but are already implemented. They are retained here as documentation of what was audited and confirmed working.

- **Kernel stack guard pages** — `KernelStack::allocate()` in `kernel/src/process/mod.rs` already unmaps a 4 KiB guard page below the 64 KiB stack. EL1 translation fault on that page produces a panic.
- **ASLR entropy pool** — `read_aslr_entropy()` in `kernel/src/loader/mod.rs` uses CNTPCT_EL0, CNTFRQ_EL0, TTBR1_EL1, and a monotonic call counter, mixed through xorshift64. Already exceeds the original task spec.
- **`select()` syscall** — Full implementation in `kernel/src/systemcalls/multiplexing.rs` (~160 lines). Supports fd_set bitmasks, blocking/non-blocking/timeout modes. `[STABLE]`.
- **FdTable thread sharing** — `Arc<SpinLock<FileDescriptorTable>>` in `Process`. `fork()` deep-clones, `clone(CLONE_THREAD)` shares the Arc. Already correct.
- **`CLOCK_REALTIME` wall clock** — PL031 RTC boot snapshot in `kernel/src/platform/qemu_virt/rtc.rs`, combined with elapsed monotonic ticks. vDSO fast path reads CNTPCT_EL0 + boot_rtc_seconds from data page.
- **`/proc/self` symlink** — Dynamic symlink in `kernel/src/fs/procfs.rs` resolving to `/proc/<current_pid>`. Per-PID directories with `status`, `maps`, `comm`. `/proc/meminfo`, `/proc/cpuinfo`, `/proc/uptime` also present.
- **`SIGALRM` / `alarm()`** — `alarm_deadline_tick` in Process struct. `sys_alarm()` sets tick deadline. Scheduler tick handler on CPU 0 scans and delivers SIGALRM.

### Tasks (remaining)

**2.1 — `copy_from_user` / `copy_to_user`** (SECURITY_DEBT §3)

The kernel has `validate_user_pointer(ptr, len) -> bool` in `kernel/src/systemcalls/mod.rs` which checks `ptr >= PAGE_SIZE`, no overflow, and `end <= USER_ADDR_LIMIT`. Most handlers already use it, but some (e.g. `sys_clock_gettime`, `sys_nanosleep`) do manual ad-hoc checks. Formalize into two canonical helpers:

```rust
fn copy_from_user(user_src: *const u8, len: usize, kernel_dst: &mut [u8]) -> Result<(), Errno>
fn copy_to_user(user_dst: *mut u8, kernel_src: &[u8]) -> Result<(), Errno>
```

Both must call `validate_user_pointer()` and return `EFAULT` on violation. Audit and replace every ad-hoc pointer validation in syscall handlers with these helpers for consistency.

**2.2 — VMA: Vec → BTreeMap** (MEMORY_DEBT implicit)

In `kernel/src/process/`, replace `Vec<MmapRegion>` (flat list, max 1024) with `BTreeMap<u64, MmapRegion>` keyed by region start address. Update `find_vma(addr)` to use `range(..=addr).next_back()` with an overlap check. Update insertions (`insert`) and removals (`remove`). Maximum VMA count: 4096 regions. Enforce at mmap time with `ENOMEM`.

**2.3 — `SCM_RIGHTS` fd passing** (KERNEL_MISC_DEBT §2)

Implement ancillary data for `sendmsg(2)` / `recvmsg(2)`. Define `struct cmsghdr` layout (identical to Linux ABI). For `SOL_SOCKET` / `SCM_RIGHTS`: sender duplicates each listed fd into a kernel transfer buffer attached to the socket message. Receiver installs each fd from the buffer into its own FdTable and returns the new fds in `recvmsg` ancillary data. Limit: 253 fds per message.

**2.4 — `MAP_SHARED` file-backed regions** (MEMORY_DEBT §2)

Anonymous `MAP_SHARED` works via `SHARED_REGION_TABLE`. The `shared` flag has been added to `MmapRegion` and is set correctly on mmap. File-backed `MAP_SHARED` demand-pages are read from the inode correctly via the page fault handler. Remaining work: add a `PageTable::remap_range_writable()` method to un-CoW shared region pages after `fork()`, and implement writeback (flush dirty shared pages to inode on `msync`/`munmap`). This requires coordinated changes to the MMU code, fork path, and page fault handler — deferred to a dedicated sub-task to avoid risk to the fork correctness.

**2.5 — `kill()` permission enforcement** (SECURITY_DEBT §4)

In `sys_kill()`: before delivering a signal, check: `sender.euid == 0 OR sender.uid == target.uid OR sender.euid == target.uid OR sender.uid == target.saved_uid`. Return `EPERM` if none match. Special case: `kill(-1, sig)` sends to all processes for which permission holds. (UID fields are added in M3; for M2, gate this check on a `kernel_has_uid_support` flag that M3 will enable.)

**2.6 — IPC inode `nlinks` field collision** (KERNEL_MISC_DEBT §7)

In `SemaphoreInode`, `SocketInode`, and `MqueueInode`: the `nlinks` field in `InodeStat` is repurposed as a table index with type discriminants encoded in the upper 32 bits (`1=sem`, `2=socket`, `3=mqueue`). This violates POSIX stat semantics. Fix: each inode already has a `table_index: usize` field — use it directly for lookups instead of encoding/decoding through `nlinks`. Set `nlinks = 1` in all IPC inode `stat()` methods.

**2.7** Tag commit `v0.3-kernel-correctness`.

> **Note:** The slab allocator task originally listed here (fixed capacity per slab)
> does not apply — the allocator already uses dynamic freelists per size class with
> no fixed capacity constant. Each slot is individually allocated from the first-fit
> heap on demand.

### Exit criteria
All 7 remaining items individually testable via QEMU. SCM_RIGHTS passes an fd across a Unix socket boundary. `kill()` from uid=1000 to uid=0 returns `EPERM`. IPC inodes report `nlinks=1` in stat output.

---

## M3 — POSIX Identity and DAC (→ v0.4)

**Goal:** Every process has a real UID/GID identity. POSIX Discretionary Access Control is enforced in the VFS. `chmod`/`chown` are fully implemented. Foundation for the Binary Permission Model is complete.

### Tasks

**3.1 — Process identity fields**

Add to the `Process` struct: `uid: u32`, `gid: u32`, `euid: u32`, `egid: u32`, `suid: u32`, `sgid: u32`, `supplemental_groups: [u32; 16]`, `ngroups: usize`, `umask: u32`. Initialize bzinit (PID 1) with all fields = 0 (root). Children inherit parent identity on `fork()`.

**3.2 — Identity syscalls** (SECURITY_DEBT §1)

Implement in `kernel/src/systemcalls/identity.rs` and mark `[STABLE]`:

- `getuid()` → `process.uid`
- `getgid()` → `process.gid`
- `geteuid()` → `process.euid`
- `getegid()` → `process.egid`
- `setuid(uid)`: if `euid == 0`, set `uid = euid = suid = uid`; else if `uid == suid`, set `euid = uid`; else `EPERM`
- `setgid(gid)`: symmetric with `setuid`
- `setreuid(ruid, euid)`: POSIX semantics (ruid or euid may be -1 = no change)
- `setregid(rgid, egid)`: symmetric
- `getgroups(size, list)`: copy `supplemental_groups[0..ngroups]` to user buffer
- `setgroups(size, list)`: `euid == 0` required; copy ≤16 groups from user buffer
- `sys_umask(mask)`: set `process.umask = mask & 0o777`, return old value

Enable the `kill()` permission check from M2.13 (`kernel_has_uid_support = true`).

**3.3 — BAFS inode uid/gid/mode** (SECURITY_DEBT §2)

Extend the BAFS on-disk inode structure (in `kernel/src/fs/bafs/`) to store `uid: u32`, `gid: u32`, `mode: u16`. Update the inode serializer and deserializer. FAT32 inodes report `uid=0`, `gid=0`, `mode=0o755` (dirs) or `mode=0o644` (files) as fixed constants.

**3.4 — DAC check in VFS** (SECURITY_DEBT §2)

Implement `fn vfs_check_access(inode_stat: &InodeStat, process: &Process, access: Access) -> Result<(), Errno>`. Logic:

1. If `process.euid == 0`: allow read/write unconditionally; for execute, check at least one execute bit is set.
2. Determine applicable bits: if `euid == uid` → owner bits (mode >> 6); elif gid match or supplemental group match → group bits (mode >> 3); else → other bits.
3. Mask applicable bits with access request (R=4, W=2, X=1). Return `EACCES` if not satisfied.

Call this in: `vfs_open()` (before returning the fd), `vfs_exec()` (check X bit), `vfs_mkdir()` (check W+X on parent), `vfs_unlink()` (check W+X on parent), `vfs_readdir()` (check R+X on directory).

Apply `umask` in `sys_open(O_CREAT)`, `sys_mkdir()`, `sys_creat()`: `effective_mode = requested_mode & ~process.umask`.

**3.5 — `chmod`, `fchmod`, `chown`, `fchown`** (SECURITY_DEBT §2)

Implement (remove stubs) in `kernel/src/systemcalls/identity.rs`:

- `sys_chmod(path, mode)`: resolve inode; require `euid == 0 || euid == inode.uid`; set `inode.mode = mode & 0o7777`; persist to BAFS.
- `sys_fchmod(fd, mode)`: same via FD.
- `sys_chown(path, uid, gid)`: `euid == 0` to change uid; `euid == 0 || (euid == inode.uid && gid in process.groups)` to change gid; persist.
- `sys_fchown(fd, uid, gid)`: same via FD.

Mark all four `[STABLE]`.

**3.6 — Setuid/setgid bits on exec**

In `elf_loader_build_image()`: if `inode.mode & 0o4000` (setuid bit) and `inode.uid != 0` — set `process.euid = inode.uid`, `process.suid = inode.uid`. If `inode.mode & 0o2000` (setgid bit) — set `process.egid = inode.gid`, `process.sgid = inode.gid`. On any `exec()`, if the new binary does not have setuid, clear `process.suid` to `process.uid`.

**3.7 — User database files**

En Bazzulto, el usuario uid=0 se llama `system`, no `root`. Esto refleja el modelo donde la escalada de privilegios se hace a través del BPM (capabilities por binario), no cambiando de usuario. `su system` existe solo para compatibilidad POSIX; el flujo normal de administración usa herramientas Tier 1 que ya tienen las capabilities correctas.

Create on `disk.img`:
- `/system/etc/passwd` (colon-delimited, 7 fields per POSIX):
  ```
  system:x:0:0:Bazzulto System:/home/system:/system/bin/bzsh
  user:x:1000:1000:user:/home/user:/system/bin/bzsh
  ```
- `/system/etc/shadow` (for v1.0: plaintext passwords, no hashing):
  ```
  system:*:0:0:99999:7:::
  user:bazzulto:0:0:99999:7:::
  ```
- `/system/etc/group`:
  ```
  system:x:0:system
  user:x:1000:user
  ```
- `/system/etc/hostname`: `bazzulto`

**3.8 — `id`, `whoami` coreutils**

`id`: print `uid=N(name) gid=N(name) groups=N(name),...`. Calls `getuid`, `getgid`, `getgroups`. Resolves names by parsing `/system/etc/passwd` and `/system/etc/group`.

`whoami`: print the username for `geteuid()`, resolved from `/system/etc/passwd`.

**3.9 — `su` — shim de compatibilidad POSIX**

`su` existe únicamente para compatibilidad con software que asume el modelo Unix clásico de escalada por UID. En Bazzulto **no es el mecanismo de administración**; las tareas del sistema se realizan mediante binarios Tier 1 que ya tienen las capabilities correctas. `su system` no bypasea el BPM: cada programa que se ejecute desde esa shell sigue pasando por `permissiond` individualmente.

Implementación: setuid-system (uid=0) binary en `/system/bin/su`. Lee el usuario objetivo del argumento (default: `system`). Parsea `/system/etc/passwd` para uid/gid. Lee `/system/etc/shadow` para la contraseña (comparación plaintext para v1.0). En match: `setuid(target_uid)`, `setgid(target_gid)`, `exec("/system/bin/bzsh", ["bzsh", "-l"])`.

El shell resultante es bzsh (Tier 1, FULL_CAPS en su propia identidad), pero los binarios que el usuario ejecute desde él siguen siendo evaluados por `permissiond` individualmente. `su system` cambia el eje DAC (uid=0 bypasea chmod/chown checks), pero el eje BPM no cambia.

**3.10** Tag commit `v0.4-posix-identity`.

### Exit criteria
`ls -l` shows uid/gid and mode. A process with `uid=1000` cannot open a `0600/uid=0` file. `chmod 000 file && cat file` returns `EACCES`. `id` and `whoami` print correct output. `su user` drops from root to uid=1000.

---

## M4 — Btrfs as Default Root Filesystem (→ v0.5)

**Goal:** The root partition uses **Btrfs** as the default system filesystem. FAT32 is retained only for the EFI System Partition. BAFS remains available as a secondary mountable filesystem but is not the root.

**Rationale:** Btrfs is a proven, production-grade filesystem with CoW, checksumming, and journaling. Bazzulto depends on the upstream Btrfs development for its root filesystem — reimplementing those guarantees from scratch in BAFS is not justified when a battle-tested solution already exists. BAFS stays as an optional secondary filesystem for experimentation and specific use cases.

### Tasks

#### Part A — Btrfs as system root

**4.1 — Implement Btrfs read-write driver**

Implement a kernel Btrfs driver in `kernel/src/fs/btrfs/` supporting: superblock parsing, chunk tree traversal, subvolume tree read, inode/extent data read, directory listing, file creation/deletion, write with CoW semantics. The driver does not need compression (zlib/lzo/zstd) or RAID support in v1.0 — single-device, uncompressed mode is sufficient.

**4.2 — `mkfs.btrfs` host utility**

Create `tools/mkfs_btrfs/` (Rust binary, host-compiled) or integrate an existing `mkfs.btrfs` into the build pipeline. Formats `disk.img` as a Btrfs volume with a root subvolume populated from the build output.

**4.3 — Update disk image pipeline**

Change the Makefile's disk assembly:

- `disk.img` (1 GiB): format with Btrfs. Partition 1 (within ISO) remains FAT32 for ESP.
- `disk2.img` (2 GiB): format with Btrfs (home partition).
- The `disk-mounts` config must declare both partitions with `fstype=btrfs`.

**4.4 — Kernel root mount change**

In `kernel_main()` or the initial mount sequence: probe block devices for Btrfs superblock magic (`_BHRfS_M` at offset 0x10040). Mount the first Btrfs partition as `/`. Mount the ESP (FAT32 partition) as `/boot/efi`.

**4.5 — Btrfs stress test**

Before tagging this milestone, run the following inside QEMU:

1. Create 50,000 files in `/data/stress/`, stat each by name, delete all in reverse order.
2. Write a 500 MiB sequential file using `dd`, compute checksum with `cksum`, read back and verify checksum.
3. Truncate the 500 MiB file to 0, verify free space is reclaimed.
4. Kill QEMU during a 100 MiB write. Restart. Verify Btrfs journal/log recovery leaves a consistent filesystem.

#### Part B — BAFS stabilization (parallel track)

**4.6 — Freeze BAFS on-disk format**

Document the format in `docs/wiki/BAFS.md`: superblock layout (magic `0x42414653_00000001`; version field; block size; journal offset; B-tree root pointer), extent B-tree node layout, inode layout (uid, gid, mode, nlink, size, atime, mtime, ctime, 8 inline extents), directory entry layout, journal record types. After this document is tagged, any breaking change to the on-disk layout requires `version = 2` and a migration tool. Version 1 volumes must always be mountable by a v1.0 kernel.

**4.7 — Fix `truncate()` extent freeing**

Already specified in M2.12. Must be completed before BAFS stress tests.

**4.8 — `mkbafs` host utility**

Create `tools/mkbafs/` (Rust binary, host-compiled). Usage: `mkbafs <image_path> --size <bytes> [--label <name>]`. Writes: superblock at block 0, initialized journal region, block allocator bitmap, root directory inode (uid=0, gid=0, mode=0o755), empty root directory B-tree.

**4.9 — `fsck.bafs` utility**

Create `tools/fsck_bafs/` (Rust binary, compiled for both host and target `aarch64-unknown-none`). Checks: superblock magic and version, all B-tree node integrity, no overlapping extents, no double-allocated blocks, inode reference count consistency, directory entry ↔ inode consistency. If the journal is dirty on open, replay it before checking. Output: `BAFS: clean` or a list of errors. `--fix` mode: move orphan inodes to `/lost+found`. Exit code 0 = clean, 1 = corrected, 2 = uncorrectable.

Install target binary to `/system/sbin/fsck.bafs` on `disk.img`.

**4.10 — BAFS mountable as optional secondary filesystem**

BAFS must be mountable on any path (e.g. `/mnt/bafs`) while Btrfs is root. This keeps BAFS functional for experimentation and specific use cases. BAFS failures do not block v1.0.

**4.11 — BAFS optional stress test**

Run the same four tests from 4.5 against a mounted BAFS partition. Run `fsck.bafs` after — must report clean. Results are documented but do not gate the release.

**4.12** Tag commit `v0.5-btrfs-root`.

### Exit criteria
`make run` boots with Btrfs root. `mount` output shows `/` on `btrfs`. All four Btrfs stress tests pass. BAFS is mountable as a secondary filesystem and `fsck.bafs` is available. BAFS stress test results are documented (pass or known issues tracked).

---

## M5 — Binary Permission Model — Tiers 1, 2 y 4 (→ v0.5.1)

**Goal:** Completar Tier 1 (system binaries, full caps). Implementar Tier 2 (policy store por hash de binario). Implementar Tier 4 con comportamiento diferenciado según contexto: prompt interactivo `[s/N]` cuando hay TTY, denegación automática sin TTY. Documentar el comportamiento de scripts (el intérprete pasa el BPM, no el script). Tier 3 (parseo de sección ELF `.bazzulto_permissions` con dialog declarativo de permisos específicos) queda post-v1.0.

### Tasks

**5.1 — Complete Tier 1**

In `kernel/src/permission/`: verify that every binary whose path is under `//system:/bin/` or `//system:/sbin/` is checked against the path-based capability list before exec. Any path outside those prefixes does not receive Tier 1 privileges. Ensure `vfs_exec()` calls the permission check and returns `EPERM` for Tier 1 violations. Document the exact path prefixes and capability grants in `docs/wiki/Permission-Model.md`.

**5.2 — Policy store on BAFS**

The Tier 2 policy store lives in `/system/policy/`. Each entry is a file named by the SHA-256 hex string (64 characters) of the binary's ELF load segments. File content (plain text, one field per line):
```
binary_path = /home/user/myapp
granted_permissions = //user:/home/**, //ram:/tmp/**
grant_scope = permanent
granted_by_uid = 0
granted_at = 1700000000
```

**5.3 — Merkle root computation**

Implement `fn compute_binary_hash(elf_load_segments: &[u8]) -> [u8; 32]` in `kernel/src/loader/`: SHA-256 over the concatenation of all `PT_LOAD` segment contents in program header order. For static binaries (v1.0), this is the only input. For dynamic binaries (added in M7), hash the binary segments then XOR-chain with each `DT_NEEDED` library's segment hash in dependency order. The result is the policy key.

**5.4 — `permissiond` service**

Create `userspace/services/permissiond/`. It:
- Reads all files under `/system/policy/` at startup and builds an in-memory hash-table: `SHA256 → permissions`
- Listens on `/dev/permissiond.sock` (Unix domain socket)
- Responds to `query(hash: [u8; 32], tty_fd: Option<Fd>, binary_path: &str)` requests:
  - `GRANTED(permissions)` — entrada en policy store con caps asignadas
  - `DENIED` — entrada en policy store marcada DENIED
  - `PROMPT_REQUIRED` — binario desconocido y hay TTY disponible
  - `AUTO_DENIED` — binario desconocido y no hay TTY disponible
- Para `PROMPT_REQUIRED`: `permissiond` escribe el prompt en el TTY del proceso solicitante, lee la respuesta, y responde `GRANTED_INHERITED` o `DENIED` al kernel
- Provides `policy_add(hash, path, permissions, scope, uid)` RPC (requires `euid == 0`)

**5.5 — Kernel Tier 2 + Tier 4 check**

In `elf_loader_build_image()`, after computing the binary hash:

1. Si `permissiond.sock` no es alcanzable (race en boot): caer a Tier 1 solamente.
2. Enviar `query(hash, tty_fd, binary_path)` a `permissiond`. El `tty_fd` es el fd del terminal controlador del proceso padre (`process.controlling_tty`); `None` si no hay TTY.
3. Manejar la respuesta:
   - `GRANTED(permissions)` → asignar `process.granted_permissions = permissions`
   - `DENIED` → abortar con `EPERM`, loguear en bzlogd
   - `GRANTED_INHERITED` → asignar `process.granted_permissions = parent.granted_permissions` (Tier 4, usuario aprobó)
   - `AUTO_DENIED` → abortar con `EPERM`, escribir en bzlogd: `"[bpm] auto-denied: {path} (no TTY available for unknown binary)"`

El kernel bloquea `exec()` mientras espera la respuesta de `permissiond`. Esto es correcto: el proceso aún no ha arrancado.

**5.6 — Pre-populate system policy**

Add a Makefile step that runs a host-side tool (`tools/policy_gen/`) to compute SHA-256 hashes of all built system binaries and generate policy files in `disk.img/system/policy/`, granting appropriate permissions for each coreutil and system service.

**5.7 — `bzpermctl` utility**

Create `userspace/src/bzpermctl/` (requires `euid == 0`):
- `bzpermctl list` — print all policy entries (hash prefix, path, permissions)
- `bzpermctl add <binary_path> <perm1,perm2,...>` — send `policy_add` RPC to `permissiond`
- `bzpermctl remove <binary_path>` — remove policy entry
- `bzpermctl verify <binary_path>` — print whether the binary has a policy entry and what it grants

**5.8 — Tier 4: prompt interactivo en TTY**

Cuando `permissiond` recibe `PROMPT_REQUIRED` (binario desconocido con TTY disponible), escribe directamente en el TTY del proceso solicitante:

```
[bazzulto] /home/user/myapp no tiene registro de permisos.
           Ejecutar con permisos heredados? [s/N]:
```

Comportamiento:
- Timeout de 30 segundos sin respuesta → tratar como `N`
- `s` o `S` → responder `GRANTED_INHERITED` al kernel; el binario corre con caps heredadas del padre
- `n`, `N`, Enter vacío, o timeout → responder `DENIED` al kernel; exec falla con `EPERM`
- El prompt bloquea el shell hasta que el usuario responde (el proceso está en mitad de `exec()`, el shell ya está esperando)

**Nota sobre scripts:** Los scripts (`.sh`, `.py`, etc.) **nunca llegan a este prompt**. El kernel hace `exec()` del intérprete (bzsh, python, etc.), no del script. El intérprete es Tier 1 o tiene entrada en policy; el script es simplemente un argumento. El confinamiento de scripts individuales dentro de un intérprete es responsabilidad de Tier 3 (post-v1.0).

**5.9 — Tier 4: auto-denegación sin TTY**

Cuando `permissiond` recibe `AUTO_DENIED` (binario desconocido sin TTY — script automatizado, servicio, pipe):
- No hay prompt
- `permissiond` envía `AUTO_DENIED` al kernel
- El kernel aborta el exec con `EPERM`
- `permissiond` escribe en bzlogd: `[bpm] auto-denied {path} hash={hash_prefix} reason=no-tty`

Esto protege contextos no interactivos: un servicio de bzinit o un cron job no puede ejecutar binarios desconocidos silenciosamente.

**5.10 — Documentar comportamiento de scripts**

Añadir sección "Scripts y el BPM" en `docs/wiki/Permission-Model.md`:

- Un script con shebang ejecutado como `./script.sh` resulta en `exec(interprete, ["interprete", "./script.sh"])`. El BPM evalúa el intérprete, no el script.
- El contenido del script hereda las caps del intérprete.
- Los scripts de servicios definidos en bzinit deben usar intérpretes Tier 1 explícitamente en el campo `exec`.
- El confinamiento por script individual (distintas caps para distintos scripts del mismo intérprete) es Tier 3, post-v1.0.

**5.11** Tag commit `v0.5.1-permission-tiers-1-2-4`.

**5.12 — Policy store integration (deferred to post-M5)**

The in-memory `PolicyStore` exists (`kernel/src/permission/policy_store.rs`) with
`lookup_best()`, `insert()`, and `load_from_text()`. Two pieces remain:

1. **Connect store to exec dispatch:** Before prompting the user, `sys_exec`
   should call `policy_store.lookup_best(hash_hex, uid)`. If a policy exists,
   skip the prompt and apply the stored permissions directly (Tier 2 fast path).
2. **Persist grants:** When permissiond receives `GrantedInherited` from the
   user, it should call a new `bpm_store_policy(hash, permissions)` syscall that
   inserts the entry into the kernel policy store. Subsequent execs of the same
   binary (same SHA-256 hash) will hit Tier 2 and skip the prompt.

This is the difference between M5 (prompt every time) and the full spec (prompt
once, remember forever).

**5.13 — permissiond TTY routing (deferred to M8)**

Currently permissiond writes prompts to stdout (display pipe). The correct
architecture: permissiond should write to the **blocked process's TTY**, not its
own stdout. This requires new kernel syscalls:

- `bpm_write_to_blocked_tty(blocked_pid, buf, len)` — kernel resolves the
  blocked process's controlling TTY and writes the prompt there.
- `bpm_read_from_blocked_tty(blocked_pid, buf, len)` — kernel reads the
  response from the same TTY.

This enables correct behavior across multiple terminals, SSH sessions (future),
and serial consoles. Deferred to M8 (Terminal Experience).

### Exit criteria
- Binario con entrada en `/system/policy/`: ejecuta sin prompt ni warning
- Binario con policy `DENIED`: EPERM inmediato
- Binario desconocido ejecutado desde terminal interactivo: muestra prompt `[yes/No]`, respuesta `No` produce EPERM, respuesta `yes` ejecuta con caps heredadas
- Binario desconocido ejecutado desde script o servicio (sin TTY): EPERM automático con entrada en bzlogd
- Script con shebang: no muestra ningún prompt (el intérprete lo absorbe)
- `permissiond` corriendo y queryable via `bzpermctl list`

---

## M6 — musl libc Integration and Test (→ v0.6)

**Goal:** musl builds against the Bazzulto syscall ABI. `libc.a` and `libc.so.6` are installed in `/system/lib/`. Six libc conformance test programs pass.

### Tasks

**6.1 — Syscall coverage audit**

For every `__syscall` invocation in `musl/src/` and `musl/arch/aarch64/`, verify the Bazzulto syscall number in the patch overlay matches `docs/wiki/System-Calls.md`. Document every musl-invoked syscall in `docs/wiki/Musl-Syscall-Coverage.md` with: musl name, Bazzulto number, implementation status. Fix any syscall that musl calls but that is currently `[RESERVED]` (implement it or map it to the correct existing syscall).

**6.2 — Finalize musl patch overlay**

The existing patch in `musl-patches/` remaps syscall numbers. Review it for completeness after the coverage audit. Verify the patch applies cleanly to the exact musl version pinned in the repo. After applying the patch, perform a full build with no errors or warnings.

**6.3 — Build musl**

Add Makefile targets:

```makefile
MUSL_SRC  := vendor/musl
MUSL_OUT  := build/musl

musl-static: $(MUSL_OUT)/lib/libc.a
musl-shared: $(MUSL_OUT)/lib/libc.so.6 $(MUSL_OUT)/lib/ld-bazzulto.so.1
```

Both targets cross-compile for `aarch64`. The static build uses `-nostdlib -freestanding`. The shared build adds `-fPIC`. The `ld-bazzulto.so.1` binary is the musl dynamic linker (`ldso/`).

**6.4 — Install to disk image**

Add to `DISK_FILES` in the Makefile:
- `$(MUSL_OUT)/lib/libc.a` → `/system/lib/libc.a`
- `$(MUSL_OUT)/lib/libc.so.6` → `/system/lib/libc.so.6`
- `$(MUSL_OUT)/lib/ld-bazzulto.so.1` → `/system/lib/ld-bazzulto.so.1`
- `$(MUSL_OUT)/include/` → `/system/include/` (headers for building C programs)

**6.5 — libc conformance test suite**

Create `tests/libc/` with six C programs, each printing `PASS` and exiting 0 on success:

- `test_stdio.c`: `printf`, `sprintf`, `fprintf(stderr)`, `fopen`/`fwrite`/`fread`/`fclose`/`fseek`/`ftell`
- `test_stdlib.c`: `malloc`/`calloc`/`realloc`/`free` at sizes 1, 64, 4096, 65536; `atoi`; `strtol`; `qsort` on 1000 elements
- `test_string.c`: `memcpy`, `memmove`, `memset`, `strcmp`, `strcpy`, `strcat`, `strstr`, `strtok_r`
- `test_signal.c`: `sigaction(SIGUSR1)`, `raise(SIGUSR1)`, verify handler ran, `sigprocmask`
- `test_time.c`: `clock_gettime(CLOCK_REALTIME)` returns epoch > 1700000000, `clock_gettime(CLOCK_MONOTONIC)` increases, `nanosleep(100ms)` sleeps ≥ 90 ms
- `test_pthread.c`: 10 threads each increment a shared counter under `pthread_mutex_t`; verify final count = 10 × increment_count

Add `make test-libc` target: builds all tests against `libc.a` (static), boots QEMU, runs each binary, checks exit codes.

**6.6** Tag commit `v0.6-musl-stable`.

### Exit criteria
`make test-libc` prints `PASS` for all six tests. `libc.so.6` and `ld-bazzulto.so.1` are present on `disk.img`. `docs/wiki/Musl-Syscall-Coverage.md` shows no uncovered gaps.

---

## M7 — Dynamic Linker (→ v0.6.1)

**Goal:** `PT_INTERP` ELF loading works. musl's `ld-bazzulto.so.1` sirve como dynamic linker. Se construye `libbsl.so.1` (BSL como shared library con C ABI) para que programas C/C++ puedan linkear contra ella dinámicamente. Los coreutils C pasan a ser dinámicos. Los binarios Rust de BSL permanecen estáticos.

**Modelo de linking en Bazzulto v1.0:**

```
Binarios Rust (bzinit, bzsh, bzdisplayd, etc.)
  → estáticos, incluyen BSL internamente, sin .so
  → no participan del dynamic linker

Programas C (coreutils, nano, programas del usuario)
  → dinámicos
  → DT_NEEDED: libc.so.6
  → DT_NEEDED: libbsl.so.1  (si usan APIs de BSL via <bazzulto/*.h>)

Programas C++ (v1.0: solo C ABI de BSL, sin stdlib C++)
  → dinámicos
  → DT_NEEDED: libc.so.6
  → DT_NEEDED: libbsl.so.1
  → sin libstdc++/libc++ (post-v1.0)
  → restricción: -fno-exceptions -fno-rtti para v1.0
```

### Tasks

**7.1 — PT_INTERP support en el kernel ELF loader** (KERNEL_MISC_DEBT §6)

In `kernel/src/loader/`, when parsing ELF program headers: if a `PT_INTERP` segment is present:

1. Read the interpreter path string (e.g., `/system/lib/ld-bazzulto.so.1`)
2. Open, load, and map the interpreter ELF into the process address space at an ASLR base above `0x4_0000_0000`
3. Map the main binary's `PT_LOAD` segments at their normal ASLR base (no saltar al entry del binario todavía)
4. Set the initial process entry point to `interpreter_load_base + interpreter_e_entry`
5. Populate the auxiliary vector:
   - `AT_PHDR`: virtual address of the main binary's program header table
   - `AT_PHNUM`: count of program headers
   - `AT_PHENT`: size of each program header entry
   - `AT_ENTRY`: original entry point of the main binary
   - `AT_BASE`: load base of the interpreter
   - `AT_EXECFN`: address of the null-terminated binary path string
   - `AT_RANDOM`: address of 16 random bytes (from ASLR pool, placed on the stack)
   - `AT_PAGESZ`: `4096`
   - `AT_HWCAP`: `0`

**7.2 — Build `libbsl.so.1` (BSL como cdylib)**

La BSL está escrita en Rust y expone una C ABI a través de los headers en `include/bazzulto/`. Para que programas C/C++ puedan linkear contra ella dinámicamente se necesita compilarla como `cdylib`.

En el `Cargo.toml` de la crate BSL correspondiente añadir:
```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

El build produce `libbsl.so` → renombrar a `libbsl.so.1` con el SONAME correcto vía `rustflags = ["-C", "link-arg=-Wl,-soname,libbsl.so.1"]`.

Añadir target en Makefile:
```makefile
BSL_SOLIB := userspace/target/$(BSL_TARGET)/release/libbsl.so
bsl-shared: $(BSL_SOLIB)
```

Exportaciones de `libbsl.so.1`: solo los símbolos declarados con `#[no_mangle]` y `extern "C"` en el código Rust. No exportar símbolos internos de Rust (usar un linker version script o `--export-dynamic` selectivo).

**7.3 — Instalar headers C/C++ de BSL**

Los headers en `include/bazzulto/` deben instalarse en el disco imagen para que programas puedan compilar contra BSL:

- `include/bazzulto/*.h` → `/system/include/bazzulto/` (C headers)
- Cualquier `.hpp` en `include/bazzulto/` → `/system/include/bazzulto/` (C++ wrappers)

Añadir a `DISK_FILES` en Makefile:
```makefile
include/bazzulto/:/system/include/bazzulto/
```

Los headers C++ de BSL deben usar `extern "C"` internamente para declarar los símbolos de `libbsl.so.1`. No exponen clases C++ ni templates — son wrappers C++ de conveniencia sobre una ABI C pura. El soporte completo de C++ stdlib (std::vector, std::string, excepciones, RTTI) es post-v1.0.

**7.4 — Instalar `libbsl.so.1` en disco imagen**

Añadir a `DISK_FILES`:
```makefile
$(BSL_SOLIB):/system/lib/libbsl.so.1
```

Crear symlink `/system/lib/libbsl.so → libbsl.so.1` (para compatibilidad con `-lbsl` en el linker).

**7.5 — Verificar resolución de paths en musl ldso**

musl's `ldso/dynlink.c` busca shared libraries en `LD_LIBRARY_PATH` y el RPATH compilado (`/system/lib`). Verificar que `vfs_open("/system/lib/libbsl.so.1")` y `vfs_open("/system/lib/libc.so.6")` resuelven correctamente desde el contexto del dynamic linker. Sin cambios de kernel esperados; es una prueba de integración.

**7.6 — Verificar tipos de relocation**

Crear `tests/dynlink/libtest.c` + `tests/dynlink/consumer.c` que ejerzan: `R_AARCH64_RELATIVE`, `R_AARCH64_GLOB_DAT`, `R_AARCH64_JUMP_SLOT`, `R_AARCH64_COPY`. Verificar que musl ldso los resuelve correctamente para ambas: `libc.so.6` y `libbsl.so.1`.

**7.7 — Test: hello world con libc + BSL**

Crear `tests/dynlink/hello_bsl.c`:
```c
#include <bazzulto/diagnostics.h>   /* BSL C header */
#include <stdio.h>                   /* musl libc    */

int main(void) {
    printf("libc ok\n");
    bzl_print_info("libbsl ok");     /* BSL C ABI call */
    return 0;
}
```
Compilar: `aarch64-linux-musl-gcc -I/system/include -o hello_bsl hello_bsl.c -lc -lbsl`
Verificar en QEMU: imprime ambas líneas, `ldd hello_bsl` muestra `libc.so.6` y `libbsl.so.1`.

**7.8 — Test: hello world C++ con BSL (C ABI only)**

Crear `tests/dynlink/hello_bsl.cpp`:
```cpp
#include <bazzulto/diagnostics.h>   /* BSL C++ wrapper header */

extern "C" int main(void) {
    bzl_print_info("C++ con BSL ok");
    return 0;
}
```
Compilar: `aarch64-linux-musl-g++ -fno-exceptions -fno-rtti -I/system/include -o hello_bsl_cpp hello_bsl.cpp -lbsl -lc`
Verificar en QEMU: el binario corre y el programa imprime la línea esperada.

Este test confirma que C++ puede consumir la ABI C de BSL sin necesitar libstdc++ ni libc++.

**7.9 — Transición coreutils a dynamic linking**

Cambiar el build de los coreutils C en el Makefile de `-static` a dinámico. Todos los coreutils C deben linkear contra `libc.so.6`. Los que usen APIs de BSL deben añadir `-lbsl`. Reconstruir `disk.img`. Verificar que todos los coreutils pasen sus tests individuales.

Los binarios Rust (`bzinit`, `bzdisplayd`, `bzsh`, `permissiond`, y demás servicios BSL) permanecen estáticos. No participan del dynamic linker.

**7.10** Tag commit `v0.6.1-dynamic-linker`.

### Exit criteria
- `hello_bsl` corre en QEMU e imprime ambas líneas; `ldd` muestra `libc.so.6` y `libbsl.so.1`
- `hello_bsl_cpp` corre en QEMU compilado con `-fno-exceptions -fno-rtti`
- Todos los coreutils C corren como dinámicos
- `stat /system/lib/libbsl.so.1` muestra entrada válida
- `stat /system/include/bazzulto/` muestra directorio con headers instalados

---

## M8 — Terminal Experience and Shell Completeness (→ v0.7)

**Goal:** bzdisplayd supports full ANSI/VT100/xterm-256color. Errors print in red; warnings in yellow. bzsh is complete for POSIX interactive use.

### Tasks

**8.1 — ANSI/VT100 state machine in bzdisplayd**

Implement a VT100 parser in `bzdisplayd` that handles:

- **SGR** (`ESC[Nm`): `0` reset, `1` bold, `4` underline, `7` reverse, `22` normal intensity, `30–37` fg colors, `40–47` bg colors, `90–97` bright fg, `100–107` bright bg, `38;5;n` 256-color fg, `48;5;n` 256-color bg, `38;2;r;g;b` true-color fg, `48;2;r;g;b` true-color bg
- **Cursor:** `ESC[A/B/C/D` (up/down/right/left), `ESC[{n}A` etc. (with count), `ESC[H` (home), `ESC[{r};{c}H` (absolute position), `ESC[s` / `ESC[u` (save/restore)
- **Erase:** `ESC[J` (erase below), `ESC[1J` (erase above), `ESC[2J` (clear screen), `ESC[K` (erase to EOL), `ESC[2K` (erase full line)
- **Mode:** `ESC[?25l` / `ESC[?25h` (hide/show cursor)
- **Query:** `ESC[6n` → reply `ESC[{row};{col}R` (cursor position report)
- **Control characters:** `\r`, `\n`, `\t`, `\b`, `\a`

The Limine framebuffer is 32bpp. True-color sequences write directly to framebuffer ARGB pixels.

**8.2 — TIOCGWINSZ**

In `sys_ioctl(TIOCGWINSZ)`: return a `struct winsize` with `ws_row` and `ws_col` computed from `framebuffer_height / glyph_height` and `framebuffer_width / glyph_width`. `ws_xpixel` and `ws_ypixel` are the raw framebuffer dimensions. This is required for nano, less, and any screen-aware program.

**8.3 — Scrollback buffer**

In `bzdisplayd`: maintain a ring buffer of the last 2000 rendered lines (as pre-rendered glyph rows, each row = one array of colored cells). On Shift+PageUp: scroll up by half a screen. On Shift+PageDown: scroll down. The scrollback never affects the PTY input path — it is display-only.

**8.4 — Color protocol for Bazzulto system tools**

In `Bazzulto.Diagnostics`:
- `print_error(msg)` → write `\e[1;31m{msg}\e[0m\n` to fd 2
- `print_warning(msg)` → write `\e[1;33m{msg}\e[0m\n` to fd 2
- `print_success(msg)` → write `\e[1;32m{msg}\e[0m\n` to fd 2
- `print_info(msg)` → write `{msg}\n` to fd 1

All Bazzulto system services and coreutils must use these functions (not raw `write` calls) for diagnostic output.

**8.5 — bzsh: automatic stderr coloring**

When bzsh runs a child process, if the child's stderr (fd 2) is a TTY (check `isatty(2)` on the shell's terminal fd), redirect the child's stderr through an internal pipe. bzsh reads from the pipe and prefixes each line with `\e[1;31m` and appends `\e[0m`. Disable via environment variable `BAZZULTO_COLOR_STDERR=0`. When the child redirects its own stderr (`2>file`), bypass this mechanism.

**8.6 — `$TERM` and terminal environment**

Set `TERM=xterm-256color` in the initial bzsh environment (sourced from `/system/etc/environment`). Set `COLORTERM=truecolor`. Do not ship a terminfo database for v1.0; bzdisplayd supports xterm-256color sequences directly.

**8.7 — bzsh: `test` / `[` builtin** (SH_DEBT §1)

Implement as a builtin per POSIX.1-2024 §2.8.3. File operators: `-e`, `-f`, `-d`, `-r`, `-w`, `-x`, `-s`, `-L`, `-p`. String operators: `-z`, `-n`, `=`, `!=`, `<`, `>`. Integer operators: `-eq`, `-ne`, `-lt`, `-le`, `-gt`, `-ge`. Logical: `!`, `-a`, `-o`. `[` requires closing `]` as the last argument; `test` does not.

**8.8 — bzsh: `printf` builtin** (SH_DEBT §2)

Implement `printf format [args...]` as a builtin. Supported specifiers: `%s`, `%d`, `%i`, `%u`, `%o`, `%x`, `%X`, `%f`, `%e`, `%g`, `%c`, `%%`. Escape sequences: `\n`, `\t`, `\r`, `\\`, `\0NNN` (octal). Width and precision modifiers.

**8.9 — bzsh: `local` variables** (SH_DEBT §4)

Add a scope stack to the bzsh variable store. Push a new frame on function entry, pop on return. `local name[=value]` declares in the current frame only; shadowed by inner scopes, invisible to caller.

**8.10 — bzsh: `trap` handlers** (SH_DEBT §6)

Maintain a per-shell `signal → handler_string` table. On signal delivery, set a pending flag. Between each command execution (after the command returns), check pending flags and evaluate the trap handler string by re-entering the parser. `trap '' SIG` ignores the signal. `trap - SIG` resets to default.

**8.11 — bzsh: `set -e` POSIX semantics** (SH_DEBT §5)

Fix errexit: suppress the exit when a failing command is in the condition of `if`, `while`, `until`, or `elif`; after `||` or `&&`; as a non-final pipeline stage; after `!` negation. Exit on any other non-zero return.

**8.12 — bzsh: globbing** (SH_DEBT §7)

Implement `glob_expand(pattern: &str, cwd: &str) -> Vec<String>` using `getdents64` on the directory portion of the pattern. Support: `*` (any string except `/`), `?` (any single character), `[abc]` (character class), `[!abc]` (negated class). Sort results. If no match, return the unexpanded literal pattern (POSIX behavior — not an error).

**8.13 — bzsh: command history and line editing** (SH_DEBT §9)

In-memory ring buffer of 1000 entries. Persistent history file: `~/.bzsh_history` (load on start, append on submit). Line editor (no external library):
- Up/Down: navigate history (`ESC[A` / `ESC[B`)
- Left/Right: move cursor within line
- Home/End: jump to start/end of line (`ESC[H` / `ESC[F`)
- Ctrl+U: clear line
- Ctrl+W: delete word before cursor
- Ctrl+L: clear screen, redraw prompt
- Backspace: delete character before cursor

**8.14 — bzsh: tab completion** (SH_DEBT §9)

On Tab: if the current word starts with `/` or `./`, complete against filesystem paths using `getdents64`. Otherwise, complete against all executables in `PATH` directories. If one match: complete immediately. If multiple matches: print them on the next line and show the longest common prefix.

**8.15 — bzsh: job control** (SH_DEBT §8)

Requires kernel support for `setpgrp`, `getpgrp`, `SIGTSTP`, `SIGCONT`, `tcsetpgrp`. Verify each is in the ABI table. Implement in bzsh:
- Each foreground command runs in a new process group; `tcsetpgrp` hands terminal control to it
- `Ctrl+Z` sends `SIGTSTP` to the foreground group; bzsh reclaims terminal with `tcsetpgrp`
- `bg [%n]`: send `SIGCONT` to job n, continue in background
- `fg [%n]`: send `SIGCONT`, call `tcsetpgrp`, wait for it
- `jobs`: list all jobs with PID, state (running/stopped), and command string

**8.16 — bzsh: `(( expr ))` arithmetic command** (SH_DEBT §3)

Detect `((` as a standalone command token. Evaluate integer arithmetic: `+`, `-`, `*`, `/`, `%`, `**`, bitwise `& | ^ ~ << >>`, comparisons `< > <= >= == !=`, logical `&& || !`, ternary `?:`. Return exit code 0 if result ≠ 0, else 1.

**8.17 — `tput` coreutil**

Add `/system/bin/tput`. Capabilities supported (direct ANSI output, no terminfo required): `clear`, `bold`, `sgr0`, `setaf N`, `setab N`, `cup row col`, `cols` (print column count), `lines` (print row count), `cnorm`, `civis`.

**8.18** Tag commit `v0.7-terminal-experience`.

### Exit criteria
All items in `docs/debts/SH_DEBT.md` have a corresponding task marked done. ANSI 256-color palette renders correctly. bzsh runs a 50-case POSIX §2 conformance test script with all cases passing. Stderr of a failing command appears in red automatically.

---

## M9 — Coreutils Completeness (→ v0.7.1)

**Goal:** Every utility listed here is in `/system/bin/` or `/system/sbin/`, passes functional tests, and exits with POSIX-correct codes.

**Existing coreutils — audit required:** Verify each of the 34 existing binaries (cat, ls, cp, mv, rm, wc, head, tail, grep, sort, uniq, cut, tr, touch, mkdir, pwd, df, ps, echo, printf, env, date, sleep, true, false, basename, dirname, cksum, diff, time, kill, shutdown, reboot) is POSIX-correct in its exit codes, option handling, and error messages. Fix any discovered deviation.

**New utilities to implement:**

**9.1 `ln`** — Hard and symbolic links. Flags: `-s` (symbolic), `-f` (force overwrite), `-n` (no-dereference). Hard links across filesystems return `EXDEV`.

**9.2 `stat`** — Print inode metadata: path, size, blocks, block size, inode number, hard links, mode (symbolic and octal), uid/gid, atime/mtime/ctime in ISO 8601. `-c FORMAT` for custom format strings.

**9.3 `chmod`** — Standalone chmod coreutil (wraps `sys_chmod`). Symbolic modes: `u+x`, `g-w`, `a=r`, `o+rw`. Octal modes. `-R` recursive.

**9.4 `chown`** — Standalone chown coreutil. Accepts `user:group`, `user`, `:group`. Resolves names from `/system/etc/passwd` and `/system/etc/group`. `-R` recursive.

**9.5 `find`** — Directory tree walk. Required predicates: `-name pattern`, `-type f|d|l|p|s`, `-maxdepth n`, `-mindepth n`, `-size [+|-]n[ckMG]`, `-newer file`, `-mtime [+|-]n`, `-user name`, `-group name`, `-perm mode`. Actions: `-print` (default), `-exec cmd {} ;`, `-exec cmd {} +`, `-delete`. Boolean: `-a`, `-o`, `!`.

**9.6 `xargs`** — Read tokens from stdin (newline-delimited by default, NUL-delimited with `-0`). Flags: `-n max_args`, `-I replace_str`. Execute command per batch.

**9.7 `tee`** — Copy stdin to stdout and files. Flag: `-a` (append).

**9.8 `sed`** — Stream editor. Commands: `s/pattern/replacement/flags` (g, i, n), `d`, `p`, `q`, `a\text`, `i\text`, `y/chars/chars/`. Flags: `-n`, `-e script`, `-f file`, `-i[suffix]`. Patterns: POSIX BRE.

**9.9 `dd`** — Block copy. Parameters: `if`, `of`, `bs`, `count`, `skip`, `seek`. Operands: `conv=notrunc,sync,noerror`. Prints transfer stats to stderr on completion.

**9.10 `tar`** — POSIX ustar format only (no compression for v1.0). Operations: `-c`, `-x`, `-t`. Flags: `-f archive`, `-v`, `-C dir`. Produces archives readable by GNU tar.

**9.11 `od`** — Octal dump. Formats: `-o` (octal), `-x` (hex), `-d` (decimal), `-c` (characters). Address base via `-A x|d|o|n`. Byte count via `-N n`.

**9.12 `strings`** — Print printable character sequences from binary files. `-n min_len` (default 4). `-t x|d|o` for byte offset.

**9.13 `tty`** — Print path of the terminal connected to stdin. Exit 1 if stdin is not a TTY.

**9.14 `stty`** — Display and set terminal attributes. No-argument form prints current settings. `stty -a` prints all. `stty sane` resets to sane defaults. Wraps `tcgetattr`/`tcsetattr`.

**9.15 `uname`** — System information. Flags: `-a`, `-s` (`Bazzulto`), `-n` (hostname), `-r` (kernel version), `-v` (build date), `-m` (`aarch64`).

**9.16 `hostname`** — Print hostname (no args) or set it (one arg). Reads/writes `/system/etc/hostname`. Calls `sys_sethostname()` (add as syscall 162, `[PROVISIONAL]`, if not already present).

**9.17 `uptime`** — Print `HH:MM:SS up HH:MM, N users, load average: 0.00, 0.00, 0.00`. Uptime from `CLOCK_MONOTONIC`. Load average always `0.00` for v1.0.

**9.18 `which`** — Search `PATH` for an executable. `-a` prints all matches.

**9.19 `sync`** — Flush all dirty BAFS buffers to block device. Calls `sys_sync()` (add as syscall 163, `[STABLE]`). BAFS driver must implement a full journal commit + block flush on this syscall.

**9.20 `id`** — Already specified in M3.8. Verify here.

**9.21 `su`** — Already specified in M3.9. Verify here.

**9.22 `clear`** — Write `\e[H\e[2J` to stdout.

**9.23 `more`** — Simple pager. SPACE = next page, ENTER = next line, `q` = quit. Uses `TIOCGWINSZ` for screen height.

**9.24 `mkfs.bafs` → `/system/sbin/mkfs.bafs`** — Thin wrapper over `mkbafs` (M4.3 target binary).

**9.25 `fsck.bafs` → `/system/sbin/fsck.bafs`** — Target binary from M4.4.

**9.26** Update `Makefile`: add all new binaries to `_BSL_BIN_MAPPINGS` or equivalent. Remove `echo` and `pwd` from `BSL_BIN_EXCLUDE` if they have been migrated to standalone coreutil sources.

**9.27** Tag commit `v0.7.1-coreutils`.

### Exit criteria
All 57 coreutils (34 existing + 23 new) present. `find . -name "*.txt" -exec grep pattern {} +` pipeline works. `sed 's/foo/bar/g' file` produces correct output. `tar -cf a.tar dir/ && tar -xf a.tar -C /tmp/` round-trips correctly.

---

## M10 — Text Editor: kibi (→ v0.8)

**Goal:** `kibi` corre en Bazzulto OS. Los usuarios pueden abrir, editar, guardar y buscar archivos desde el terminal. kibi es un editor Rust de ~1000 líneas sin dependencias externas de crates — se integra directamente en el workspace BSL y usa el mismo toolchain del resto del userspace.

**Por qué kibi sobre nano:** kibi no tiene dependencia de ncurses ni de ninguna librería externa. Escribe ANSI sequences directamente al terminal. La única interfaz de kernel que necesita (`tcgetattr`/`tcsetattr` + `TIOCGWINSZ`) ya está implementada como parte de M2 y M8. El porte es esencialmente añadirlo al workspace y compilar.

### Tasks

**10.1 — Añadir kibi al workspace BSL**

Añadir kibi como crate en `userspace/Cargo.toml`:
```toml
[workspace]
members = [
    ...
    "src/kibi",
]
```

Crear `userspace/src/kibi/` con el source de kibi pinado a un commit estable. Verificar que el `Cargo.toml` de kibi no declara dependencias externas de crates (solo `std` equivalente via BSL). Si las declara, eliminarlas y sustituir por las primitivas de BSL equivalentes.

**10.2 — Adaptar kibi a `aarch64-unknown-none`**

kibi usa `std` de Rust. Como el target del workspace BSL es `aarch64-unknown-none` (no_std), verificar qué partes de `std` usa kibi y sustituirlas:

- I/O de terminal (`stdin`/`stdout` raw) → syscalls `read`/`write` directos via BSL
- `tcgetattr`/`tcsetattr` → `Bazzulto.System` wrappers (syscalls 61/62, ya STABLE)
- `TIOCGWINSZ` → `sys_ioctl(TIOCGWINSZ)` vía BSL (implementado en M8.2)
- `File::open`/`File::read`/`File::write` → `Bazzulto.IO`
- `SIGWINCH` para resize → registrar handler via `sigaction` (syscall 21, ya STABLE)

Si la adaptación de `std` → `no_std` resulta demasiado invasiva, compilar kibi como crate independiente con target `aarch64-unknown-linux-musl` (con `std` via musl) en lugar de `aarch64-unknown-none`. En ese caso linkea contra `libc.so.6` dinámicamente.

**10.3 — Features habilitadas para v1.0**

kibi por defecto soporta: abrir/editar/guardar, búsqueda (Ctrl+F), syntax highlighting vía archivo de configuración, numeración de líneas. Para v1.0 habilitar:

- Syntax highlighting para: `.rs` (Rust), `.c`/`.h` (C), `.sh` (shell), `.md` (Markdown)
- Numeración de líneas activada por defecto
- Indicador de archivo modificado en la status bar
- Mensaje de ayuda en la barra inferior: `Ctrl-S guardar | Ctrl-Q salir | Ctrl-F buscar`

**10.4 — Archivo de configuración**

Crear `/system/etc/kibi/config` en el disco imagen:
```
line_numbers = true
tab_width = 4
```

Crear `/system/etc/kibi/syntax.d/` con los archivos de syntax highlighting para Rust, C, shell y Markdown.

**10.5 — Instalar kibi**

Añadir a `DISK_FILES`:
```makefile
$(BSL_BIN_DIR)/kibi:/system/bin/kibi
$(BSL_BIN_DIR)/kibi:/system/bin/edit   # alias conveniente
```

**10.6 — Test kibi**

Verificar en QEMU:
- `kibi /system/etc/hostname` — abre el archivo, muestra contenido con número de línea
- Editar texto, `Ctrl+S` — guarda; `cat` el archivo confirma el contenido nuevo
- `Ctrl+F pattern` — búsqueda resalta la primera coincidencia
- Redimensionar terminal (`SIGWINCH`) — kibi redibuja correctamente
- `Ctrl+Q` con cambios sin guardar — pide confirmación antes de salir
- `Ctrl+Q` sin cambios — sale sin prompt

**10.7** Tag commit `v0.8-text-editor`.

### Exit criteria
`kibi /system/etc/hostname` abre el archivo, permite editar, guarda con Ctrl+S y sale con Ctrl+Q sin corrupción del terminal.

---

## M11 — bzinit: Full Service Definitions (→ v0.8.1)

**Goal:** bzinit arranca un conjunto completo de servicios del sistema en orden de dependencias. Los servicios están supervisados, logueados y se reinician según su política. El hostname se configura directamente en bzinit al arrancar — no requiere un daemon dedicado ya que v1.0 no tiene red.

### Tasks

**11.1 — `bzlogd` — System log daemon**

Create `userspace/services/bzlogd/`. Listens on `/dev/log` (Unix socket). Receives log messages from any process. Writes to `/data/logs/bazzulto.log` with format `[ISO8601] [LEVEL] [pid:name] message\n`. Log rotation: at 8 MiB, rename to `bazzulto.log.1`, keep 3 rotations. Expose `bzlog(level, msg: &str)` in BSL (`Bazzulto.Diagnostics`) as the canonical log call. Restart policy: `always`; si falla 5 veces en 60 s → system halt.

**11.2 — `bztimed` — System time service**

Create `userspace/services/bztimed/`. On startup: reads PL031 RTC (via `/dev/rtc0` or direct MMIO), calls `sys_clock_settime(CLOCK_REALTIME, seconds)` to set the kernel wall clock. Runs a re-sync every 30 minutes. Restart policy: `on-failure`, max 3 attempts.

**11.3 — Hostname en bzinit (sin servicio dedicado)**

`bzhostnamed` no existe como servicio independiente. Sin red, el hostname solo necesita estar disponible para el prompt del shell y para `uname -n`. bzinit lo configura directamente durante su secuencia de init, antes de arrancar ningún servicio:

```rust
// En bzinit main(), antes del loop de servicios:
let hostname = fs::read_to_string("/system/etc/hostname")
    .unwrap_or_else(|_| "bazzulto".to_string());
sys_sethostname(hostname.trim());
```

El coreutil `hostname` (M9.16) permite leer y cambiar el hostname en runtime; los cambios persisten escribiendo directamente a `/system/etc/hostname`. No hay socket, no hay daemon.

**11.4 — `bztty` — TTY allocator**

Create `userspace/services/bztty/`. Allocates PTY pairs for virtual terminals (tty0 through tty3). Gives tty0 to `bzdisplayd` at startup. Provides a socket API for requesting additional PTYs. Handles Ctrl+Alt+F1..F4 key combinations by signaling `bzdisplayd` to switch the active terminal. Restart policy: `on-failure`, max 5 attempts.

**11.5 — `permissiond`**

Already specified in M5.4. Integrate as a standard bzinit service with service file. Restart policy: `always`; si falla 3 veces en 60 s → reboot (daemon de seguridad crítico).

**11.6 — `bzdisplayd`**

Already exists. Update its service definition to declare: `depends_on = ["bzlogd", "permissiond", "bztimed"]`. Restart policy: `on-failure`, max 5 times; on all failures → exec emergency shell directly on framebuffer.

**11.7 — `bzsh`**

Already exists. Update service definition: `depends_on = ["bztty"]`. Restart policy: `always` (so a new shell spawns when the user types `exit` or the shell crashes).

**11.8 — Boot graph**

```
bzinit main() — configura hostname desde /system/etc/hostname
     │
     ├─► Level 0 (paralelo): bzlogd, bztimed
     ├─► Level 1 (tras bzlogd): permissiond
     ├─► Level 2 (tras permissiond + bztimed): bzdisplayd
     ├─► Level 3 (tras bzdisplayd): bztty
     └─► Level 4 (tras bztty): bzsh
```

bzinit espera a que cada nivel complete (socket ready o exit 0 para one-shots) antes de arrancar el siguiente.

**11.9 — Service health checks**

Para servicios con socket Unix (`bzlogd`, `permissiond`, `bztty`): bzinit envía `PING` cada 10 s, espera `PONG` en 3 s. Tres fallos consecutivos → restart según política.

Para one-shots (`bztimed`): el health check es el exit code del proceso.

**11.10 — `bzctl` extension**

Extend `userspace/src/bzctl/` with:
- `bzctl status` — state (running/stopped/failed/restarting), PID y uptime de cada servicio
- `bzctl restart <service>` — restart via socket de control en `/dev/bzinit.sock`
- `bzctl stop <service>` — SIGTERM + wait + mark stopped
- `bzctl logs <service>` — últimas 50 líneas de `/data/logs/bazzulto.log` filtradas al servicio

**11.11** Tag commit `v0.8.1-bzinit-services`.

### Exit criteria
Los 6 servicios arrancan en orden de dependencias. `bzctl status` los muestra todos running. `kill -9 $(pidof bzdisplayd)` → bzinit lo reinicia en menos de 10 s. `/data/logs/bazzulto.log` tiene entradas con timestamp para cada arranque de servicio. `uname -n` devuelve el hostname de `/system/etc/hostname`.

---

## M12 — Boot Splash and Font System (→ v0.9)

**Goal:** The Bazzulto logo appears on the framebuffer from the moment the kernel takes control. A progress bar fills as services start. Fonts are managed by a daemon and the terminal renders at the configured size.

### Tasks

**12.1 — Bazzulto logo asset**

Create the Bazzulto OS logo as a source PNG in `assets/logo.png` (256×256, RGBA). Add a build script (`kernel/build.rs`) that converts it to a raw RGBA byte array and embeds it as `const LOGO_RGBA: &[u8]` in the kernel binary via `include_bytes!()`.

**12.2 — Early kernel splash**

In `kernel_main()`, immediately after the Limine framebuffer request is satisfied and before any other subsystem:

1. Clear the framebuffer to `0x0D1117` (dark navy, 32bpp ARGB)
2. Scale the 256×256 LOGO_RGBA to a center-aligned region: `min(framebuffer_width, framebuffer_height) / 3` pixels, bilinear scaling
3. Blit the scaled logo to the center of the framebuffer
4. Below the logo, render `"Bazzulto OS 1.0"` using the embedded 8×16 bitmap font in white
5. Continue with normal kernel init

Kernel log messages (printk-equivalent) must not overwrite the splash framebuffer after this point. Buffer all kernel log output to a ring buffer; flush to `/data/logs/kernel.log` once VFS is available.

**12.3 — `bzsplash` — animated boot splash service**

Create `userspace/services/bzsplash/`. This is the FIRST service started by bzinit. It:
- Opens the framebuffer via the Bazzulto framebuffer API
- Re-renders the Bazzulto logo (from `/system/share/logo.rgba`) centered on the screen
- Below the logo renders a 640-wide × 8-tall progress bar (empty, dark gray background)
- Listens on `/dev/bzsplash.sock` for messages:
  - `PROGRESS phase_name` — advance progress bar by 1/(total_phases) steps; update phase text
  - `DONE` — animate fade-out (16 frames, 60 Hz: multiply each pixel's RGB by (16-frame)/16), then `exit(0)`
- After exit, bzdisplayd takes exclusive ownership of the framebuffer

**12.4 — bzinit splash integration**

Update bzinit to:
1. Start `bzsplash` before all other services (Level -1); wait for its socket to appear (max 2 s)
2. After each service at each level starts, send `PROGRESS <service_name>` to bzsplash
3. After bzsh is confirmed running, send `DONE` to bzsplash

Phase names to display:
- `"Initializing logging"` (bzlogd)
- `"Setting system time"` (bztimed)
- `"Loading permissions"` (permissiond)
- `"Starting display"` (bzdisplayd)
- `"Allocating terminals"` (bztty)
- `"Ready"` (bzsh)

**12.5 — Suppress Limine boot menu**

Update `limine.cfg`: set `TIMEOUT=0` and `QUIET=YES`. The Limine selection menu must not appear during normal boot.

**12.6 — Font directory structure**

Create on `disk.img`:
```
/system/fonts/
  JetBrainsMono/
    JetBrainsMono-Regular.ttf
    JetBrainsMono-Bold.ttf
    JetBrainsMono-Italic.ttf
    JetBrainsMono-BoldItalic.ttf
```
Update `DISK_FILES` in the Makefile to include all four variants.

**12.7 — Font configuration file**

Create `/system/etc/fonts.conf` on `disk.img`:
```
terminal_font         = JetBrainsMono
terminal_font_weight  = Regular
terminal_font_size    = 14
terminal_bold_font    = JetBrainsMono-Bold
terminal_italic_font  = JetBrainsMono-Italic
```

**12.8 — Font cache format**

Define a binary font cache format. Each cache file lives at `/data/cache/fonts/<Family>-<size>px.fcache`. Header: magic `0x42464343` ("BFCC"), version `1`, codepoint count, glyph data offset. Per-glyph entry: codepoint (u32), bitmap width (u8), bitmap height (u8), advance_x (i8), bitmap data (width × height bytes, 8-bit alpha).

**12.9 — `fontmanager` daemon upgrade**

Extend the existing `fontmanager` binary (currently 280 L in `userspace/`) into a daemon:
- On startup: scan `/system/fonts/` recursively; for each TTF file, rasterize all printable ASCII + Latin-1 codepoints at `terminal_font_size` from `fonts.conf`; write the cache to `/data/cache/fonts/`
- Expose a Unix socket at `/dev/fontmgr.sock` with RPC: `get_glyph(family, size, codepoint)` → bitmap + metrics
- Handle `RELOAD` message: rescan fonts and rebuild cache (triggered by `fontmanager reload`)
- CLI mode (no daemon): `fontmanager rebuild`, `fontmanager list`, `fontmanager set <family> <size>`

Add `fontmanager` as a bzinit service at Level 0 (parallel with bzlogd). `bzdisplayd` gains `depends_on = ["fontmanager"]`. `bzsplash` uses the embedded kernel font; it does not depend on `fontmanager`.

**12.10 — bzdisplayd glyph source**

Change bzdisplayd to query `/dev/fontmgr.sock` for glyph bitmaps rather than embedding a font directly. On startup, `bzdisplayd` calls `get_glyph` for each printable character to warm a local glyph cache. `TIOCGWINSZ` returns dimensions computed from glyph advance_x and height from the loaded font.

**12.11** Tag commit `v0.9-splash-and-fonts`.

### Exit criteria
Boot shows the splash within 2 s of QEMU start. Progress bar fills with each service name. Terminal appears after fade-out. `fonts.conf` change followed by `fontmanager reload` and `bzdisplayd` restart shows the new font size.

---

## M13 — BSL 1.0 API Freeze (→ v0.9.1)

**Goal:** The public API of the five frozen BSL modules is documented, stable, and tagged 1.0.0. Placeholder modules are removed.

### Tasks

**13.1 — Remove placeholder modules**

Delete `Bazzulto.Crypto`, `Bazzulto.Net`, `Bazzulto.Storage` from the BSL workspace (`userspace/Cargo.toml`). These must not appear in public `use` paths or documentation.

**13.2 — Document frozen modules**

For every public symbol in `Bazzulto.System`, `Bazzulto.IO`, `Bazzulto.Display`, `Bazzulto.Concurrency`, `Bazzulto.Diagnostics`: add a Rust doc comment (`///`) stating: purpose, parameter semantics, return value, error conditions, thread safety. Zero `pub` symbols may remain undocumented.

**13.3 — Complete `Bazzulto.Concurrency`**

If `RwLock<T>` has any unimplemented methods, complete them using futex. `Condvar` must implement `wait`, `notify_one`, `notify_all` backed by `FUTEX_WAIT` / `FUTEX_WAKE`.

**13.4 — Remove `FileSystemWatcher` from `Bazzulto.IO`**

This was a stub. Delete it entirely from the public API.

**13.5 — Version metadata**

In the BSL workspace root `Cargo.toml`: `version = "1.0.0"`. Export `pub const BSL_VERSION: &str = "1.0.0"` from the library root. Update `sys_uname()` to return `"1.0"` in the release field.

**13.6 — Stability annotation**

Annotate each public function in frozen modules with one of: `// Stability: stable` (frozen forever) or `// Stability: provisional` (may change in v1.x patch releases but number is frozen). All `stable` functions have complete doc comments and no `todo!()`/`unimplemented!()` bodies.

**13.7 — BSL integration test**

Create `tests/bsl/` with one test per frozen module. Each test compiles against BSL, exercises the primary API functions, and exits 0 on success. Add `make test-bsl` target.

**13.8** Tag commit `v0.9.1-bsl-freeze`.

### Exit criteria
`cargo doc` succeeds with zero warnings. `grep -r 'todo!\|unimplemented!' userspace/src/bsl/` returns no results in frozen modules. `BSL_VERSION == "1.0.0"`. BSL integration tests pass.

---

## M14 — Integration Testing — v1.0-rc (→ v1.0-rc)

**Goal:** The full system test matrix passes inside QEMU. All tests in `tests/run_all.sh` exit 0. The system is in a releasable state.

### Tasks

**14.1 — Test suite location**

Create `tests/v1.0/`. Every test is a POSIX shell script. Each prints `[PASS] description` or `[FAIL] description` and exits 0 (pass) or 1 (fail). `tests/run_all.sh` runs all scripts in `tests/v1.0/`, prints a summary, and exits 0 iff all pass.

**14.2 — Kernel and syscall tests**

- `test_syscalls.sh`: invoke every `[STABLE]` syscall; verify return value and correct errno
- `test_clock.sh`: `CLOCK_REALTIME` returns epoch > 1_700_000_000; `CLOCK_MONOTONIC` increases monotonically; `nanosleep(100ms)` sleeps ≥ 90 ms
- `test_memory.sh`: mmap + munmap; fork with CoW (parent and child see independent copies); MAP_SHARED anonymous (parent and child see shared writes); large allocation (128 MiB) and free
- `test_signals.sh`: SIGCHLD on child exit; SIGALRM fires within ±100 ms of alarm(1); SIGINT cancels a blocked `read()`; `trap` handler runs after `kill -USR1 $$`
- `test_vfs.sh`: create/read/write/seek/append/truncate/unlink/mkdir/rmdir/rename/symlink/hardlink/readlink on BAFS
- `test_permissions.sh`: chmod 000 blocks non-root read; chmod 644 allows read; setuid binary runs as root euid; `kill()` to unrelated PID returns EPERM

**14.3 — Filesystem tests**

- `test_bafs_stress.sh`: create/stat/delete 1000 files; verify `fsck.bafs` clean after
- `test_bafs_journal.sh`: kill QEMU during write; restart; `fsck.bafs --fix` reports clean; file is either fully written or absent
- `test_bafs_truncate.sh`: create 50 MiB file; truncate to 0; verify free block count restored

**14.4 — Shell conformance tests**

- `test_bzsh_posix.sh`: 50-case script covering builtins, globbing, pipelines, redirections, arithmetic, `test`/`[`, job control, `trap`, history, tab completion (non-interactive via `--rcfile`)

**14.5 — IPC tests**

- `test_pipes.sh`: write 1 MiB through a pipe; verify content; close write end and verify read end returns EOF
- `test_unix_sockets.sh`: SOCK_STREAM connect/send/recv; SOCK_DGRAM; `SCM_RIGHTS` passes an open file descriptor across a socket boundary
- `test_semaphores.sh`: `sem_wait` blocks until `sem_post` from a child; value decrements and increments correctly
- `test_mqueue.sh`: `mq_send` + `mq_receive` preserves message content and ordering

**14.6 — Coreutils tests**

`test_coreutils.sh`: one functional test per coreutil (57 total). Each invocation checks exit code and critical output content.

**14.7 — Permission model tests**

- `test_perm_tier1.sh`: a system binary from `/system/bin/` runs without policy prompt
- `test_perm_tier2.sh`: a binary with a matching entry in `/system/policy/` runs without warning
- `test_perm_tier4.sh`: a binary with no policy entry produces a warning on stderr and runs

**14.8 — Service tests**

- `test_boot_services.sh`: at the time of test execution, `bzctl status` shows all 7 services as running
- `test_service_restart.sh`: `kill -9 $(pidof bzdisplayd)` → wait 10 s → `bzctl status bzdisplayd` shows running

**14.9 — Dynamic linker tests**

`test_dynlink.sh`: run `hello_dynamic`; run `cat /system/etc/hostname`; verify both print expected output.

**14.10 — musl conformance**

`test_libc.sh`: run the six test binaries from M6.5; all must print PASS.

**14.11 — nano test**

`test_nano.sh`: use expect/heredoc to drive nano non-interactively (write a test file, invoke nano with a script that opens the file, appends a line, saves with Ctrl-O, exits with Ctrl-X, then verify the file on disk contains the appended line).

**14.12 — Final pre-release checks**

Verify:
- `grep -r 'unimplemented!\|todo!\|panic!.*stub' kernel/src/ userspace/src/` → zero results reachable from userspace
- Btrfs root filesystem consistent after the full test run
- `fsck.bafs` on any mounted BAFS partition → clean
- All 162 `[STABLE]` syscalls exercised at least once in the test run
- No kernel panic logged in `/data/logs/kernel.log` during the test run

**14.13** Tag commit `v1.0-rc`.

### Exit criteria
`tests/run_all.sh` exits 0. No kernel panics. Btrfs root consistent. `fsck.bafs` clean on BAFS partitions (if mounted).

---

## M15 — ISO Packaging and v1.0 Release (→ v1.0)

**Goal:** `make iso` produces `bazzulto-1.0.iso`, a UEFI-bootable ISO that boots in both QEMU and VirtualBox 7.x AArch64 to a functional shell.

### Tasks

**15.1 — ISO layout**

```
/EFI/BOOT/BOOTAA64.EFI     — Limine UEFI stub
/boot/bazzulto.elf          — kernel ELF
/boot/limine.cfg            — boot config
/boot/limine.sys            — Limine system file (if required by xorriso target)
/disk.img                   — Btrfs root filesystem image
/disk2.img                  — Btrfs home filesystem image
```

**15.2 — Limine boot config**

`/boot/limine.cfg`:
```
TIMEOUT=1
QUIET=YES

:Bazzulto OS 1.0
    PROTOCOL=limine
    KERNEL_PATH=boot():/boot/bazzulto.elf
    MODULE_PATH=boot():/disk.img
    MODULE_PATH=boot():/disk2.img
```

**15.3 — `make iso` target**

Add a dedicated `iso` target (separate from `all`) to the Makefile. Dependency chain:
1. `kernel` (cargo build release)
2. `bsl` (cargo build release)
3. `musl-shared` (build libc.so.6 + ld-bazzulto.so.1)
4. `disk` (format disk.img with Btrfs, populate)
5. `disk2` (format disk2.img with Btrfs)
6. `iso` (xorriso command producing `bazzulto-1.0.iso`)

xorriso command must produce a valid El Torito UEFI bootable image. Verify with `file bazzulto-1.0.iso` showing `ISO 9660 CD-ROM filesystem data`.

ISO size must not exceed 3 GiB.

**15.4 — QEMU boot verification**

Verify with:
```
qemu-system-aarch64 \
    -M virt -cpu cortex-a57 -m 2048 \
    -bios /path/to/edk2-aarch64-code.fd \
    -cdrom bazzulto-1.0.iso \
    -nographic
```
Boot must reach the bzsh prompt. All 7 services must start. The test suite in `/system/tests/` (copied from `tests/v1.0/` into disk.img) must pass.

**15.5 — VirtualBox AArch64 boot verification**

VirtualBox 7.1+ supports AArch64 on Apple Silicon hosts. VM configuration:
- Architecture: ARM (AArch64)
- RAM: 2048 MiB
- CPU: 2 vCPUs
- Firmware: EFI
- Display: VMSVGA
- Optical drive: `bazzulto-1.0.iso`

Boot must reach bzsh prompt. Document exact VM configuration in `docs/virtualbox.md`.

**15.6 — Version embedding**

In `kernel_main()`, write to the framebuffer / console on boot:
```
Bazzulto OS 1.0 (aarch64) [BUILD_DATE]
```
where `BUILD_DATE` is embedded via `env!("BUILD_DATE")` set in `build.rs` at compile time.

**15.7 — Release notes**

Create `docs/release/v1.0.md`:
- List of all stable syscalls
- List of all coreutils
- BSL 1.0 stable module list
- Known limitations (no network, single-core, no package manager, dynamic linking for C only)
- How to run: QEMU invocation and VirtualBox configuration

**15.8** Tag commit `v1.0`.

### Exit criteria
`make iso` succeeds without errors. `bazzulto-1.0.iso` boots in QEMU to bzsh prompt. `bazzulto-1.0.iso` boots in VirtualBox 7.1 AArch64 to bzsh prompt. `tests/run_all.sh` (run inside the ISO boot) exits 0.

---

## v1.0 Definition of Done

A commit is tagged `v1.0` only when all of the following are simultaneously true:

- [ ] Milestones M1–M15 are tagged in git history
- [ ] `tests/run_all.sh` exits 0 on the ISO build
- [ ] Btrfs root filesystem consistent after the test run; `fsck.bafs` clean on BAFS partitions (if mounted)
- [ ] `bazzulto-1.0.iso` boots to bzsh in QEMU and VirtualBox 7.x AArch64
- [ ] No `unimplemented!()`, `todo!()`, or `panic!("stub")` in any code path reachable from userspace
- [ ] All `[STABLE]` syscalls in `docs/wiki/System-Calls.md` have a passing test case
- [ ] `docs/wiki/System-Calls.md`, `docs/wiki/BAFS.md`, `docs/wiki/Permission-Model.md`, and `docs/wiki/Musl-Syscall-Coverage.md` are complete and accurate
- [ ] `docs/release/v1.0.md` is written and accurate
- [ ] BSL version is `"1.0.0"` with no `TODO` in any frozen module
- [ ] `docs/virtualbox.md` contains verified VirtualBox boot instructions
