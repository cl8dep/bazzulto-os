# Planned Features

## FD Capabilities (Fuchsia-inspired)

**Priority:** Medium — implement after VFS is complete and multi-process is stable.
**Area:** `include/bazzulto/virtual_file_system.h`, `kernel/filesystem/virtual_file_system.c`, `kernel/arch/arm64/systemcall/systemcall.c`

### Motivation

Today, any process that holds an open FD can do everything that FD allows. If a process passes an FD to a child via `spawn`, the child inherits the same rights with no restriction. There is no way to say "you can read this file but not write it, and you cannot pass it to anyone else."

Fuchsia solves this with capability handles: rights are sealed into the handle by the kernel and cannot be escalated. Bazzulto will adopt this model on top of the existing Unix-compatible FD interface.

### Design

Add a `rights` field to `file_descriptor_t`:

```c
// Capability rights — bitmask stored in every file descriptor.
#define FD_RIGHT_READ      (1 << 0)  // Can call read()
#define FD_RIGHT_WRITE     (1 << 1)  // Can call write()
#define FD_RIGHT_SEEK      (1 << 2)  // Can call seek()
#define FD_RIGHT_DUPLICATE (1 << 3)  // Can duplicate this FD (dup syscall)
#define FD_RIGHT_TRANSFER  (1 << 4)  // Can pass this FD to another process

typedef struct {
    fd_type_t            type;
    const struct ramfs_file *file;
    size_t               offset;
    uint32_t             rights;   // ← new: capability bitmask
} file_descriptor_t;
```

The kernel checks rights on every syscall before dispatching:

```c
// In sys_write():
if (!(fds[fd].rights & FD_RIGHT_WRITE)) return -EPERM;

// In sys_seek():
if (!(fds[fd].rights & FD_RIGHT_SEEK))  return -EPERM;
```

### New Syscalls Needed

```c
/// Duplicate an FD with equal or fewer rights.
/// x0 = source fd, x1 = rights mask (subset of source rights).
/// Returns new fd, or -1 if source lacks FD_RIGHT_DUPLICATE or rights exceed source.
#define SYS_DUP  10

/// Transfer an FD to another process with equal or fewer rights.
/// x0 = target pid, x1 = source fd, x2 = rights mask.
/// Returns the fd number assigned in the target process, or -1 on error.
/// Requires FD_RIGHT_TRANSFER on source fd.
#define SYS_SEND_FD  11
```

### Behavior

- `open()` grants full rights by default (READ | WRITE | SEEK | DUPLICATE | TRANSFER).
- `open(..., O_RDONLY)` grants READ | SEEK | DUPLICATE | TRANSFER — no WRITE.
- A process can only **reduce** rights when duplicating or transferring — never escalate.
- stdin/stdout/stderr get READ | WRITE but no SEEK, DUPLICATE, or TRANSFER.

### Example

```c
// Parent opens a config file for reading:
int fd = open("/etc/config", O_RDONLY);
// fd rights: READ | SEEK | DUPLICATE | TRANSFER

// Create a restricted copy with only READ — no seek, no passing it further:
int locked_fd = dup(fd, FD_RIGHT_READ);
// locked_fd rights: READ only

// Spawn a child and give it the locked fd:
send_fd(child_pid, locked_fd);
// Child can only call read() — seek(), write(), dup(), send_fd() all return -EPERM
```

### Compatibility

Existing code using `open`/`read`/`write`/`close`/`seek` is unaffected. Rights are additive by default — programs that do not use capabilities behave exactly as before. Capabilities only restrict when explicitly used.

### What This Is Not

This is not a full Fuchsia-style capability system. Fuchsia replaces all OS primitives with handles. Bazzulto keeps Unix FDs as the primary interface and adds rights as a security layer on top — closer to Capsicum (FreeBSD) than Fuchsia.

### References

- Fuchsia handles and rights: `fuchsia.dev/fuchsia-src/concepts/kernel/handles`
- Capsicum (FreeBSD capability model): `man4.freebsd.org/capsicum`
- Current FD implementation: `include/bazzulto/virtual_file_system.h`

---

## VFS with Explicit-Scheme Paths

**Priority:** High — design decision that must be made before VFS is implemented.
**Area:** `kernel/filesystem/`, `include/bazzulto/virtual_file_system.h`

### Motivation

Unix mounts everything under a single `/` root and hides the origin of files behind mount points. This makes it impossible to tell from a path alone whether a file lives on disk, in RAM, on the network, or in a virtual filesystem. Windows uses drive letters (`C:\`) which expose the device but not its type.

Bazzulto uses **explicit-scheme paths**: the scheme prefix identifies the type of resource before any path resolution occurs. The origin is never ambiguous.

### Path Format

```
//<scheme>:<authority>/<path>
```

| Component | Meaning |
|---|---|
| `scheme` | Type of filesystem or resource |
| `authority` | Device index, label, or hostname (optional) |
| `path` | File path within that resource |

### Defined Schemes

| Scheme | Meaning | Examples |
|---|---|---|
| `ram` | In-memory filesystem (ramfs) | `//ram:/tmp/scratch.txt` |
| `disk` | Persistent block device | `//disk:0/home/user/file.txt` |
| `system` | Read-only OS partition | `//system:/bin/shell` |
| `data` | User data partition | `//data:/home/user/foto.jpg` |
| `usb` | Removable device | `//usb:0/transfer/doc.pdf` |
| `net` | Network resource | `//net:192.168.1.1/share/file` |
| `proc` | Per-process virtual filesystem | `//proc:42/memory` |
| `pipe` | Named pipe | `//pipe:/ipc/logger` |

### Multiple Disks

When more than one block device is present, the authority field disambiguates:

```
//disk:0/...      → first disk (by enumeration order)
//disk:1/...      → second disk
//disk:main/...   → disk with label "main"
//disk:sda/...    → disk by device name
```

The kernel maintains a **device registry** that maps authority strings to mounted block devices. The VFS resolves the authority before calling into the filesystem driver.

### Comparison with Unix and Windows

| | Unix | Windows | Bazzulto |
|---|---|---|---|
| Local file | `/home/user/file` | `C:\Users\file` | `//disk:0/home/user/file` |
| Temp file | `/tmp/scratch` | `%TEMP%\scratch` | `//ram:/tmp/scratch` |
| Network | `/mnt/share/file` | `\\server\share\file` | `//net:server/share/file` |
| Process info | `/proc/42/maps` | N/A | `//proc:42/maps` |

Unix hides the origin. Windows exposes the device but not the type. Bazzulto makes both explicit in every path.

### VFS Resolution Algorithm

```
parse_scheme(path)
  → look up scheme in scheme_registry[]
  → parse authority (device index or label)
  → look up device in device_registry[]
  → call device->filesystem->open(remaining_path)
```

The syscall layer never sees device details — it passes the full path to `vfs_open()` and gets back an fd.

### Compatibility Note

Programs that open files using these paths are not portable to Linux/macOS. For portability, provide a POSIX compatibility layer that maps `/tmp` → `//ram:/tmp`, `/` → `//system:/`, etc. This layer is optional and can be added later.

---

## Typed Inodes

**Priority:** Medium — implement when VFS layer is built.
**Area:** `include/bazzulto/virtual_file_system.h`, `kernel/filesystem/`

### Motivation

In Unix (ext2/ext4), the type of a filesystem node (regular file, directory, symlink, device, socket) is encoded in the upper bits of the `mode` field — a historical accident. The type is mixed with permission bits in a single `uint16_t`, requiring bitmask extraction to read (`S_ISDIR(mode)`, `S_ISLNK(mode)`, etc.).

Bazzulto defines inode type as an explicit, strongly-typed field separate from permissions.

### Design

```c
typedef enum {
    INODE_TYPE_FILE,              // Regular file
    INODE_TYPE_DIRECTORY,         // Directory
    INODE_TYPE_SYMLINK,           // Symbolic link
    INODE_TYPE_PIPE,              // Anonymous pipe
    INODE_TYPE_NAMED_PIPE,        // Named pipe (//pipe: scheme)
    INODE_TYPE_DEVICE_BLOCK,      // Block device (disk, usb)
    INODE_TYPE_DEVICE_CHAR,       // Character device (uart, keyboard)
    INODE_TYPE_NETWORK_ENDPOINT,  // Socket as a first-class inode
    INODE_TYPE_PROCESS,           // //proc:PID — process as a filesystem node
} inode_type_t;

typedef struct {
    uint32_t       inode_number;
    inode_type_t   type;          // explicit — not encoded in permissions
    uint16_t       permissions;   // rwxrwxrwx only — no type bits
    uint16_t       owner_uid;
    uint32_t       size_bytes;
    uint32_t       time_created;
    uint32_t       time_modified;
    uint32_t       time_accessed;
    uint16_t       hard_link_count;
    void          *private_data;  // filesystem-specific (block list, symlink target, etc.)
} inode_t;
```

### Benefits over Unix

- `inode.type` is always readable without bitmask arithmetic
- Adding a new type does not change the permissions field or break existing permission checks
- `INODE_TYPE_NETWORK_ENDPOINT` and `INODE_TYPE_PROCESS` are first-class types, not hacks bolted onto regular files
- The `void *private_data` field lets each filesystem store what it needs without changing the shared inode struct

---

## Extensible Inode Metadata

**Priority:** Medium — implement alongside or after typed inodes.
**Area:** `kernel/filesystem/`

### Motivation

Ext2/ext4 inodes have a fixed set of fields. Adding new metadata (content type, encryption flag, thumbnail hash) requires either changing the on-disk format and migrating all data, or using the `xattr` mechanism which was designed as an afterthought.

Bazzulto inodes support **extended attributes** as a first-class feature, not an add-on.

### Design

Each inode carries a list of key-value pairs alongside its builtin fields:

```c
typedef struct {
    char     key[64];
    uint8_t *value;
    size_t   value_length;
} inode_attribute_t;

typedef struct {
    // ... builtin fields (see Typed Inodes) ...
    inode_attribute_t *extended_attributes;
    size_t             extended_attribute_count;
} inode_t;
```

### Example Attributes

```
inode 1042  (foto.jpg)
├── [builtin]  size = 2457600, permissions = rw-r--r--
├── [extended] "content_type"   = "image/jpeg"
├── [extended] "thumbnail_hash" = "sha256:abc123..."
└── [extended] "encrypted"      = "false"
```

### Syscalls

```c
/// Read an extended attribute by key.
/// Returns value length, or -1 if not found.
int64_t getxattr(int fd, const char *key, void *buf, size_t buf_len);

/// Set or replace an extended attribute.
/// Returns 0 on success, -1 on error.
int setxattr(int fd, const char *key, const void *value, size_t value_length);

/// Remove an extended attribute.
int removexattr(int fd, const char *key);
```

### References

- macOS Extended Attributes: `man getxattr`
- Linux xattr: `man attr`

---

## Inode Versioning (Copy-on-Write History)

**Priority:** Low — implement after mutable filesystem and VFS are stable.
**Area:** `kernel/filesystem/`

### Motivation

On current filesystems, every `write()` modifies data in place. There is no way to recover a previous version of a file without a separate backup tool. ZFS solves this at the dataset level with snapshots. Bazzulto proposes per-file versioning at the inode level.

### Design

Each `write()` that modifies an existing inode creates a new inode version. The directory entry always points to the latest version. Previous versions are retained until explicitly pruned or a version limit is reached.

```
//disk:0/config.txt        → version 3 (current)
//disk:0/config.txt@2      → version 2
//disk:0/config.txt@1      → version 1 (original)
```

The `@N` suffix is resolved by the VFS path parser before calling into the filesystem driver. Filesystem drivers that do not support versioning return `-ENOSYS` for `@N` paths; the VFS does not impose versioning on drivers that opt out.

### Version Retention Policy

Configured per mount point:

```c
typedef struct {
    uint32_t max_versions;       // 0 = unlimited, N = keep last N versions
    uint64_t max_age_seconds;    // 0 = no age limit
} version_policy_t;
```

### Use Cases

- Configuration file rollback without a separate backup
- Crash recovery: the previous version of a partially-written file is always available
- Audit trail for sensitive files

### References

- ZFS copy-on-write and snapshots: `openzfs.github.io/openzfs-docs`
- Btrfs per-file snapshots: `btrfs.wiki.kernel.org`

---

## Path-Level Capability Restrictions

**Priority:** Medium — implement alongside FD Capabilities.
**Area:** `kernel/filesystem/`, `kernel/arch/arm64/systemcall/`

### Motivation

FD Capabilities (see above) restrict what a process can do with an already-open file descriptor. Path-Level Capabilities restrict which paths a process is allowed to open in the first place.

### Design

Each process has an optional **path whitelist**: a list of path prefixes the process is permitted to open. If the whitelist is non-empty, any `open()` call for a path outside the whitelist returns `-EACCES` before the filesystem driver is consulted.

```c
// At spawn time, parent can restrict child to a subtree:
spawn("//system:/bin/shell", argv, (spawn_options_t){
    .allowed_path_prefixes = { "//ram:/sandbox/", NULL },
});
// Child can only open paths under //ram:/sandbox/ — nothing else.
```

This is enforced in the VFS layer before scheme resolution, so it applies uniformly to all filesystem types.

### Relationship to FD Capabilities

| Layer | What it restricts |
|---|---|
| Path-Level Capabilities | Which paths can be opened (before open) |
| FD Capabilities | What can be done with an open fd (after open) |

Both layers are independent and composable.

---

## Dynamic Libraries (.so / shared objects)

**Priority:** Low — implement after static libc, mmap, and a full VFS with disk support.

### Motivation

Currently all userspace programs statically link every library they use. Each ELF binary carries its own copy of `string.c`, `stdio.c`, etc. As the number of programs grows this wastes RAM (multiple copies of the same code in memory) and disk space. Dynamic libraries solve this by loading one shared copy into memory and mapping it into every process that needs it.

### Prerequisites (in order)

1. **`mmap` / `munmap` syscalls** — needed to map `.so` files into process address space without copying.
2. **Position Independent Code (PIC)** — `.so` files must work at any load address. Requires compiling with `-fPIC` instead of the current `-fno-pic`.
3. **Full VFS with disk** — `.so` files must live on a real filesystem that the dynamic linker can search at runtime (`/lib`, `/usr/lib`).
4. **Dynamic linker (`ld.so`)** — a special ELF binary the kernel launches before `_start`. It reads the dependency list from the ELF, locates each `.so` on disk, maps them with `mmap`, and resolves all symbol addresses via the PLT/GOT before handing control to the program.

### How it works

```
Kernel loads program ELF
  → sees PT_INTERP segment pointing to /lib/ld.so
  → maps ld.so into the process
  → jumps to ld.so entry point

ld.so runs:
  → reads DT_NEEDED entries (list of required .so files)
  → finds each .so on disk
  → mmap()s them into the process address space
  → fills the GOT (Global Offset Table) with real symbol addresses
  → calls the program's _start
```

### Key data structures in ELF

| Structure | Purpose |
|---|---|
| `PT_INTERP` segment | Path to the dynamic linker (`/lib/ld.so`) |
| `DT_NEEDED` entries | List of `.so` dependencies |
| PLT (Procedure Linkage Table) | Stub per imported function — jumps through GOT |
| GOT (Global Offset Table) | Array of pointers filled by `ld.so` at load time |

### Complexity

The dynamic linker is the hardest single component in this list. musl's `ldso` is ~3000 lines; glibc's is far larger. SerenityOS implemented theirs over several months.

**Recommended approach for Bazzulto:** implement a minimal `ld.so` that handles only `BAZZULTO_1.0` versioned symbols, ignoring GNU_HASH, versioned dependencies, and lazy binding initially. Lazy binding and full GNU ABI compatibility can come later.

### References

- ELF specification: `refspecs.linuxfoundation.org/elf/elf.pdf`
- musl dynamic linker source: `git.musl-libc.org/cgit/musl/tree/ldso`
- How the dynamic linker works (LWN): `lwn.net/Articles/631631`
- Current ELF loader: `kernel/loader/elf_loader.c`

---

## Runtime Keyboard Layout Switching

**Priority:** Medium — implement after VFS is functional.
**Dependencies:** VFS (to read `.bkm` files from `//system:/etc/keymaps/`).
**Area:** `kernel/drivers/keyboard/keyboard.c`, `kernel/drivers/keyboard/keymap.c`

### Current State

The keyboard driver loads an embedded US QWERTY keymap at init time. There is
no way to change the layout at runtime. The `.bkm` files in
`resources/keymaps/standard/querty/` are available (us, us-intl, es, latam) but
cannot be loaded without a filesystem.

### Implementation Plan

1. Add a syscall `SYS_SET_KEYMAP` that accepts a path to a `.bkm` file.
2. The kernel reads the file via VFS, parses it with `keymap_parse()`, and
   replaces `active_keymap`.
3. Add a shell command `keymap <name>` that calls `SYS_SET_KEYMAP` with the
   path `//system:/etc/keymaps/<name>.bkm`.
4. Store the active layout name in `//system:/etc/keyboard.conf` so it persists
   across reboots.

### Example

```bash
keymap latam       # switches to Latin American layout
keymap us-intl     # switches to US International with dead keys
```

### References

- Keymap format: `docs/wiki/Keymaps.md`
- Available layouts: `resources/keymaps/standard/querty/`

