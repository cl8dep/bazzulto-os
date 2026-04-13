# bzinit v1.0 — Technical Debt

Items deferred from the v1.0 implementation. Each entry describes what was
deferred, why, and what needs to be in place before it can be done.

---

## errno support in libc_compat

**What**: `libc_compat/src/fs.rs` returns `-1` on error but does not set `errno`.

**Why deferred**: No thread-local storage (TLS) or process-global errno variable
exists in the runtime yet.

**Prerequisite**: A `BazzultoAllocator`-backed errno cell, or kernel support for
a per-process errno page.

---

## bzctl start / stop via signals

**What**: `bzctl start <name>` and `bzctl stop <name>` to dynamically manage
running services.

**Why deferred**: Signals carry no payload. A secondary IPC channel is needed
(e.g. a named pipe or a kernel-managed control fd) so bzctl can identify which
service to affect.

**Prerequisite**: Named pipe support in VFS, or a `//proc:bzinit/ctl` write fd
(analogous to the existing state fd).

---

## bzctl logs <name>

**What**: Show per-service stdout/stderr output.

**Why deferred**: Requires capturing each service's stdout/stderr at spawn time
into a pipe, then storing the pipe read-end in `ServiceState`. Not complex, but
omitted to keep v1.0 scope tight.

**Prerequisite**: None — pure userspace change. Can be done when bzinit's spawn
path is extended to redirect fds before calling exec.

---

## bzctl timeline / graph / why

**What**: Historical view of service start/stop events; dependency graph
visualization; explanation of why a service is in its current state.

**Why deferred**: Requires an event history ring buffer in bzinit (timestamp,
event type, service name). No standardized output format defined yet.

**Prerequisite**: Ring buffer allocation (trivially done with Vec); a stable
text format for the timeline; bzctl command dispatch (currently hard-coded to
status).

---

## POSIX symlinks at boot (real symlink creation)

**What**: Create actual filesystem symlinks (`/bin` → `/system/bin`, etc.) at
boot instead of doing path translation in userspace.

**Why deferred**: The VFS and ramfs layers do not support symlinks or writable
directories in v1.0. Translation in `libc_compat` is the v1.0 workaround.

**Prerequisite**: Phase 9 VFS (inodes, writable directories, symlink inode type).

---

## mkdir / rmdir / rename in libc_compat

**What**: `libc_compat/src/fs.rs` returns `-1` for `mkdir`, `rmdir`, `rename`,
and `opendir`. These are stubs only.

**Why deferred**: Ramfs does not support directory creation or renaming. These
operations are meaningless until Phase 9 VFS.

**Prerequisite**: Phase 9 VFS.

---

## execv / execve argument forwarding

**What**: `execv` and `execve` in `libc_compat` ignore `argv` and `envp`.
The kernel `exec` syscall in v1.0 accepts only a path.

**Why deferred**: Passing argv/envp to a new process image requires either a
kernel ABI extension (extra registers or a stack-based convention) or a shared
memory handoff protocol. Both are non-trivial.

**Prerequisite**: Kernel `exec` extended to accept an `argv` array pointer.

---

## Per-user and per-app service directories

**What**: Load service files from `/home/user/.config/services/` and
`/system/share/bazzukto/services/` in addition to `/config/bazzukto/services/`.

**Why deferred**: Requires directory enumeration (readdir) which is not yet
supported on ramfs. bzinit v1.0 uses a flat LIST syscall workaround.

**Prerequisite**: Phase 9 VFS readdir.

---

## Boot snapshot (persistent state across reboots)

**What**: Serialize the service state at shutdown and restore it on next boot
to detect boot loops.

**Why deferred**: Requires reliable disk write path (sync, fsync equivalents)
and a stable serialization format.

**Prerequisite**: FAT32 write path; `bz_fsync` syscall or equivalent.

---

## vDSO migration for existing C binaries

**What**: `userspace/library/systemcall.S` encodes `svc #N` immediates directly.
These binaries are not vDSO-aware and will break if syscall numbers are
renumbered.

**Why deferred**: The C kernel is being phased out. Migrating existing C
binaries is lower priority than completing the Rust kernel.

**Prerequisite**: Complete the C→Rust kernel migration; then update
`systemcall.S` to use the vDSO stubs from `Bazzulto.System/c_stubs/vdso_stubs.S`.

---

## App sandbox / capability enforcement

**What**: Restrict what syscalls and paths each app can access based on a
manifest file in `/apps/<name>/`.

**Why deferred**: Requires a kernel capability model (not yet designed).

**Prerequisite**: Kernel capability enforcement infrastructure.

---

## Dynamic BDL loader

**What**: A dynamic linker that resolves BDL (Bazzulto Dynamic Library)
dependencies at process load time using `config/bazzukto/linker.toml`.

**Why deferred**: All v1.0 binaries are statically linked. Dynamic linking
requires both the loader and a stable BDL ABI.

**Prerequisite**: Stable BSL ABI version; ELF PLT/GOT support in the loader.

---

## Directory.watch() (inotify equivalent)

**What**: Notify a process when a directory's contents change.

**Why deferred**: Kernel inotify-equivalent not designed.

**Prerequisite**: Kernel event source for filesystem modifications; a new
syscall or polling mechanism.
