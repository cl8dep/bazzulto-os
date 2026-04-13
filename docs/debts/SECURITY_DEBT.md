# Security and Permissions — Pending

## Status: NOT IMPLEMENTED

The kernel currently has no access control. Any process can open any file, send
signals to any other process, and map arbitrary memory. This must be addressed
before the OS can be considered multi-user or production-safe.

---

## 1. UID / GID per process

### Current state
`sys_getuid()` / `sys_getgid()` return 0 unconditionally. There are no uid/gid
fields in the `Process` struct.

### Required changes

Add to `Process` (`process/mod.rs`):
```rust
pub uid: u32,   // real user ID
pub gid: u32,   // real group ID
pub euid: u32,  // effective user ID (used for permission checks)
pub egid: u32,  // effective group ID
```

Initialise all to 0 (root) for the init process. Children inherit from parent on
`fork()`. `exec()` checks the ELF file's set-uid bit and updates `euid` if set.

New syscalls:
- `getuid()` → `process.uid`
- `getgid()` → `process.gid`
- `geteuid()` → `process.euid`
- `getegid()` → `process.egid`
- `setuid(uid)` → allowed if `euid == 0`; sets `uid = euid = uid`
- `setgid(gid)` → allowed if `egid == 0`; sets `gid = egid = gid`

Reference: POSIX.1-2017 §8.1 (User and Group IDs).

---

## 2. File permission bits in VFS inodes

### Current state
`InodeStat` has a `mode: u32` field but it is never checked during `open()`,
`read()`, or `write()`.

### Required changes

`InodeStat.mode` should use standard POSIX bit layout:
```
Bits 15–12: file type (regular, directory, symlink, etc.)
Bits  8– 6: owner permissions (r=4, w=2, x=1)
Bits  5– 3: group permissions
Bits  2– 0: other permissions
```

In `sys_open()`, after resolving the inode, add a permission check:
```rust
fn check_permission(inode: &dyn Inode, process: &Process, access: u32) -> bool {
    let stat = inode.stat();
    let mode = stat.mode;
    // Determine which permission bits to check (owner / group / other).
    let bits = if process.euid == stat.owner_uid {
        (mode >> 6) & 0x7
    } else if process.egid == stat.owner_gid {
        (mode >> 3) & 0x7
    } else {
        mode & 0x7
    };
    (bits & access) == access // access: 4=read, 2=write, 1=execute
}
```

Root (euid=0) bypasses all permission checks except execute on files with no
execute bit set.

New syscalls:
- `chmod(path, mode)` → update `inode.mode` (only owner or root)
- `chown(path, uid, gid)` → update `inode.owner_uid` / `inode.owner_gid` (root only)
- `fchmod(fd, mode)` → same as chmod but on open fd
- `fchown(fd, uid, gid)` → same as chown but on open fd

Add `owner_uid: u32` and `owner_gid: u32` to `InodeStat`.

---

## 3. `copy_from_user` / `copy_to_user` with full validation

### Current state
Syscalls that receive user pointers (e.g. `sys_write(buf_ptr, len)`) validate
that `buf_ptr + len` does not overflow u64, but do not verify that the range lies
within the user address space and is mapped.

### Required changes

Define the user address space boundary:
```rust
const USER_ADDRESS_SPACE_END: u64 = 0x0000_FFFF_FFFF_F000; // top of lower half
const PAGE_SIZE: u64 = 4096;
```

Implement two helpers in `syscall/mod.rs`:

```rust
/// Validate that [ptr, ptr+len) is fully within user space and non-null.
fn validate_user_range(ptr: u64, len: usize) -> bool {
    if ptr < PAGE_SIZE { return false; }  // NULL and low page forbidden
    if len == 0 { return true; }
    let end = ptr.checked_add(len as u64).unwrap_or(u64::MAX);
    end <= USER_ADDRESS_SPACE_END
}

/// Copy `len` bytes from user pointer `src` into `dst`.
/// Returns Err(-EFAULT) if the range is invalid.
unsafe fn copy_from_user(dst: &mut [u8], src: u64) -> Result<(), i64> {
    if !validate_user_range(src, dst.len()) { return Err(-14); } // EFAULT
    core::ptr::copy_nonoverlapping(src as *const u8, dst.as_mut_ptr(), dst.len());
    Ok(())
}

/// Copy `len` bytes from `src` into user pointer `dst`.
unsafe fn copy_to_user(dst: u64, src: &[u8]) -> Result<(), i64> {
    if !validate_user_range(dst, src.len()) { return Err(-14); }
    core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len());
    Ok(())
}
```

Apply these helpers to every syscall that dereferences a user pointer. Currently
the most exposed paths are:
- `sys_read` / `sys_write` (buffer pointer + length)
- `sys_open` (path pointer)
- `sys_mmap` (hint address)
- `sys_clone` (child stack pointer)
- `sys_futex` (uaddr)
- All new IPC syscalls (socket address, message buffers)

For stronger guarantees, walk the page table to verify the range is actually
mapped before dereferencing. This prevents a process from passing a pointer to
a mapped-but-not-yet-faulted CoW page and causing a kernel page fault.

---

## 4. Signal delivery to other processes — kill() permission check

### Current state
`sys_kill(pid, sig)` sends a signal to any PID unconditionally.

### Required change
A process may send a signal to another if:
- The sender's `euid == 0` (root), OR
- The sender's `uid` or `euid` matches the target's `uid` or `euid`.

Add this check in `scheduler::send_signal_to()` or in `sys_kill()`:
```rust
if sender_euid != 0 && sender_uid != target_uid && sender_euid != target_uid {
    return Err(-1); // EPERM
}
```

Reference: POSIX.1-2017 §2.8.2 (Signal Concepts, permission to send signals).

---

## 5. `seccomp` — syscall filtering (future)

A mechanism for a process to restrict its own syscall surface. Used by browsers,
container runtimes, and sandboxes. Implementation requires:
- A per-process BPF (or simplified rule list) filter.
- Checked in `syscall::dispatch()` before invoking the handler.
- `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...)` to install the filter.

This is deferred until the basic UID/GID permission model is in place.

---

## Implementation order

1. UID/GID fields + `getuid`/`getgid`/`setuid`/`setgid` (no enforcement yet)
2. `copy_from_user` / `copy_to_user` validation across all syscall paths
3. File mode bits + permission check in `sys_open`
4. `chmod` / `chown` / `fchmod` / `fchown`
5. `kill()` permission check
6. `setuid` bit on ELF exec
7. `seccomp` (future)
