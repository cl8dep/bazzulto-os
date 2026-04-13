# Miscellaneous Kernel Debt — Pending

## 1. `select()` syscall

### Current state
`epoll_create1` / `epoll_ctl` / `epoll_wait` (syscalls 64–66) and a basic `poll()`
are implemented. The classic POSIX `select()` is not.

### Why it matters
Many POSIX-compliant programs and older libraries use `select()` directly. The
`fd_set` bitmap approach is less efficient than epoll but required for full
compatibility.

### Required change

```rust
// sys_select(nfds, readfds_ptr, writefds_ptr, exceptfds_ptr, timeout_ptr) -> i64
//
// fd_set is a 128-byte bitmap (1024 bits, one per fd).
// Iterate bits 0..nfds, check readiness, write results back.
```

`fd_set` layout (Linux/POSIX compatible):
```rust
#[repr(C)]
struct FdSet {
    fds_bits: [u64; 16],  // 16 × 64 = 1024 bits
}
```

`select()` can be implemented as a thin wrapper over the existing `epoll` readiness
logic: iterate the fd_set bits, call `check_fd_readiness()` for each set bit.

**Files:** `syscall/mod.rs` (new `sys_select()`), reuse `fs/epoll.rs` helpers.

---

## 2. `SCM_RIGHTS` — file descriptor passing over Unix sockets

### Current state
Unix domain sockets (`ipc/socket.rs`) support `send()` and `recv()` for data. The
`sendmsg()` / `recvmsg()` syscalls with `SCM_RIGHTS` control messages (which
allow passing open file descriptors between processes) are not implemented.

### Why it matters
`SCM_RIGHTS` is used by: D-Bus, Wayland, `systemd`, `ssh-agent`, container
runtimes. Without it, Unix socket-based IPC is limited to byte streams.

### Required change

New syscalls:
- `sendmsg(sockfd, msghdr_ptr, flags)` → send data + optional cmsg
- `recvmsg(sockfd, msghdr_ptr, flags)` → receive data + optional cmsg

`msghdr` layout (matches Linux):
```rust
#[repr(C)]
struct MsgHdr {
    msg_name:       u64,    // optional socket address
    msg_namelen:    u32,
    msg_iov:        u64,    // *iovec array
    msg_iovlen:     u64,    // number of iovec elements
    msg_control:    u64,    // *cmsghdr (for SCM_RIGHTS)
    msg_controllen: u64,
    msg_flags:      i32,
}

#[repr(C)]
struct CmsgHdr {
    cmsg_len:   u64,   // total length including data
    cmsg_level: i32,   // SOL_SOCKET = 1
    cmsg_type:  i32,   // SCM_RIGHTS = 1
    // followed by array of i32 fds
}
```

For SCM_RIGHTS: extract the fd array from the cmsg, duplicate each fd into the
receiver's fd table (analogous to `dup()`), write the new fd numbers into the
received cmsg.

**Files:** `ipc/socket.rs`, `syscall/mod.rs`.

---

## 3. `SIGALRM` / `alarm()`

### Current state
The kernel has a tick counter and `nanosleep()` that puts a process to sleep for N
ticks. `SIGALRM` (signal 14) is not delivered automatically.

### Why it matters
`alarm(seconds)` is used by many programs for timeouts (e.g. `timeout` command,
network clients, `system()`). It is required for POSIX compliance.

### Required change

Add `alarm_tick: u64` to `Process` (0 = no alarm set).

`sys_alarm(seconds) -> u32`:
- Return remaining seconds of previous alarm (0 if none).
- `alarm_tick = current_tick + seconds * ticks_per_second`.
- If seconds == 0: cancel the alarm.

In `Scheduler::wake_sleeping_processes()` (called each tick on CPU 0):
- For each process where `alarm_tick != 0 && current_tick >= alarm_tick`:
  - Deliver `SIGALRM` (signal 14).
  - Set `alarm_tick = 0`.

**Files:** `process/mod.rs`, `scheduler/mod.rs`, `syscall/mod.rs`.

---

## 4. FD table reference counting for threads

### Current state
`clone(CLONE_VM | CLONE_THREAD)` copies the parent's FD table into the child
(`clone_thread()` copies the `fd_table` field). Changes to FDs in one thread are
not visible to other threads in the same thread group.

### Why it matters
POSIX requires that all threads in a thread group share the same file descriptor
table. A `close(fd)` in one thread must close the fd for all threads. Without this,
multi-threaded programs that share fds (the common case) will malfunction.

### Required change

Wrap `FdTable` in `Arc<Mutex<FdTable>>` (using the existing `SpinLock`):

```rust
// In Process:
pub fd_table: Arc<SpinLock<FdTable>>,
```

`fork()` clones the Arc (new independent copy): `Arc::new(SpinLock::new(parent_fd_table.lock().clone()))`.

`clone_thread()` clones the Arc pointer (shared table): `Arc::clone(&parent.fd_table)`.

All FD operations must acquire the lock before accessing the table.

**Files:** `process/mod.rs`, `scheduler/mod.rs` (fork + clone_thread), `syscall/mod.rs` (all fd operations).

---

## 5. `/proc/self` symlink

### Current state
`/proc/<pid>/status`, `/proc/<pid>/maps` exist. There is no `/proc/self` symlink.

### Why it matters
Programs commonly use `/proc/self/exe`, `/proc/self/fd/`, `/proc/self/maps` to
inspect themselves without knowing their own PID. Required for tools like `lsof`,
`gdb`, and many language runtimes.

### Required change

Add `InodeType::Symlink` to `inode.rs`. Add a `SymlinkInode { target: String }`
that implements `Inode`. `read_at` returns the symlink target as bytes; `stat()`
returns `mode = 0o120777`.

In `procfs`, on `lookup("self")`: return a `SymlinkInode` whose target is
`format!("/proc/{}", current_pid.index)`.

Path resolution in `vfs_resolve()` must follow symlinks (currently it does not).

**Files:** `fs/inode.rs`, `fs/procfs.rs`, `fs/mount.rs` (symlink resolution in `vfs_resolve`).

---

## 6. ELF dynamic linker support (`PT_INTERP`)

### Current state
`loader/mod.rs` handles `ET_EXEC` (static executables) and `ET_DYN` (PIE). It
does not handle `PT_INTERP`, which specifies the dynamic linker path (e.g.
`/lib/ld-musl-aarch64.so.1`). Dynamically-linked binaries will fail to load.

### Why it matters
Most binaries compiled with a standard toolchain are dynamically linked. Supporting
them requires either: (a) implementing `PT_INTERP` loading so the dynamic linker
runs first, or (b) requiring all Bazzulto userspace binaries to be statically
linked (the current approach).

### Required change (option a)

In `loader/mod.rs`, after parsing ELF headers:
1. If a `PT_INTERP` segment exists, read the interpreter path.
2. Resolve the interpreter path in VFS.
3. Load the interpreter ELF into a separate address range (above `USER_IMAGE_BASE + 0x40000000`).
4. Set the process entry point to the interpreter's entry, not the binary's `e_entry`.
5. Pass the original binary's load address in `AT_BASE` aux vector entry.

This requires implementing the auxiliary vector (`AT_*` entries on the initial
user stack), which `ld.so` reads to locate the loaded binary.

**Files:** `loader/mod.rs`, `process/mod.rs` (aux vector setup).

---

## 7. `nlinks` field collision in IPC inodes

### Current state
`SemaphoreInode`, `SocketInode`, and `MqueueInode` all store their table index in
`InodeStat::nlinks` to avoid needing `downcast_ref` (not available in `no_std`
without `Any`). This creates a potential false positive: a socket with index 5
and a semaphore with index 5 produce the same `nlinks` value, and the wrong table
may be consulted.

### Required change

Use `InodeStat::inode_number` to encode both the subsystem discriminant and the
index:
```
inode_number = (discriminant << 32) | index
```
Where `discriminant`:
- 0 = regular inode (allocated by `alloc_inode_number()`)
- 1 = semaphore
- 2 = Unix socket
- 3 = message queue
- 4 = epoll instance

The syscall layer decodes `(inode_number >> 32)` to select the correct table, and
`(inode_number & 0xFFFF_FFFF)` as the index.

**Files:** `ipc/sem.rs`, `ipc/socket.rs`, `ipc/mqueue.rs`, `fs/epoll.rs`, `syscall/mod.rs`.

---

## 8. `clock_gettime(CLOCK_REALTIME)` — wall clock

### Current state
`clock_gettime(CLOCK_MONOTONIC)` works (tick counter × TICK_INTERVAL_MS).
`CLOCK_REALTIME` returns the same monotonic value, which is wrong: it should
return seconds since Unix epoch (1970-01-01T00:00:00Z).

### Required change

QEMU `virt` includes a PL031 RTC (Real Time Clock) at physical address `0x09010000`.
- Map the PL031 MMIO region in `platform_init()`.
- Read the `RTCDR` register (offset 0x000, 32-bit count of seconds since
  2000-01-01T00:00:00Z for PL031, or since epoch depending on QEMU config).
- Store the base real-time value at boot: `rtc_base_seconds = rtc_read()`.
- `CLOCK_REALTIME = rtc_base_seconds + (current_tick × TICK_INTERVAL_MS / 1000)`.

Reference: PL031 TRM DDI 0224 §3.3.

**Files:** New `platform/qemu_virt/rtc.rs`, `platform/qemu_virt/mod.rs`, `syscall/mod.rs`.
