# App Permission Model

**Priority:** High — must be designed before app packaging and the ELF loader are finalized.
**Area:** `kernel/filesystem/`, `kernel/arch/arm64/systemcall/`, `include/bazzulto/permissions.h`

## Motivation

Linux gives processes coarse-grained access to everything by default. Restricting
an app requires configuring AppArmor or SELinux — external policy files in
specialized syntax, invisible to the user, rarely audited.

Android and iOS introduced the idea that apps declare what they need, the user
approves it, and the OS enforces it. But Android permissions are categorical
(`INTERNET`, `CAMERA`) — all or nothing. There is no way to say "only my backend
server", only "the entire internet".

Bazzulto unifies the filesystem path model with the permission model. Because
every resource in the system — files, network, devices, IPC — is addressed by a
`//scheme:authority/path` route, permissions are expressed as path patterns.
This gives Android-style UX with fine-grained, auditable, human-readable control.

## Core Idea

A permission is a path pattern. Granting a permission means allowing `open()`
calls whose path matches that pattern.

```
//net:**                      → any internet access
//net:*.my-backend.com/**     → only my backend server
//dev:cam:0                   → front camera only
//user:/documents/**          → read/write user documents
//user:/documents/*.pdf       → only PDF files in documents
```

The kernel enforces this at the VFS layer before any filesystem driver is
consulted. If the path does not match any granted permission, `open()` returns
`-EACCES` regardless of inode permissions.

## Permission Declaration

There are two app models in Bazzulto depending on whether the app has a UI:

### Shell binaries (current — no manifest)

Command-line programs (`shell`, `ls`, `cat`, `echo`, etc.) are plain ELF
binaries with no manifest. They are system programs that run with a fixed,
kernel-defined permission set. No bundle, no XML, no package — just the ELF.

```
//system:/bin/shell.elf     ← plain binary, no manifest
//system:/bin/ls.elf
//system:/bin/cat.elf
```

### UI applications (future — XML manifest)

Graphical apps are distributed as bundles. The bundle contains a `manifest.xml`
alongside the ELF and assets. The manifest is XML for consistency with
established app ecosystems (Android, Windows AppX) and because it is easily
parsed by installers, app stores, and tooling without a custom format.

```
Editor.app/
├── manifest.xml        ← app identity, permissions, metadata
├── bin/
│   └── editor.elf
├── icons/
│   ├── icon_64.png
│   └── icon_256.png
└── assets/
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<app>
  <identity>
    <name>Editor</name>
    <id>com.arael.editor</id>
    <version>1.2.0</version>
    <description>A simple text editor</description>
    <author>Arael Espinosa</author>
    <icon>icons/icon_256.png</icon>
    <entry>bin/editor.elf</entry>
    <min-os-version>0.3.0</min-os-version>
  </identity>

  <permissions>
    <pattern>//net:*.my-backend.com/**</pattern>
    <pattern>//user:/documents/**</pattern>
    <pattern>//ram:/tmp/editor/**</pattern>
  </permissions>
</app>
```

The ELF binary contains no embedded permissions — it is pure code. The kernel
reads `manifest.xml` at load time. The declared patterns are the **maximum**
the app can ever hold. The user may grant a subset.

**Why manifest over ELF section:**
- Readable by humans and tools without an ELF parser
- The package manager can audit permissions before installation
- The launcher can display name, icon, and description without executing anything
- Permissions can be inspected independently of the binary

## User Approval

On first launch the kernel presents the permission list to the user via the
system UI:

```
"browser" is requesting access to:

  ● Internet — any website         (//net:**)
  ● Downloads folder               (//user:/downloads/**)
  ● Temporary files                (//ram:/tmp/browser/**)

[Allow All]  [Deny]  [Customize...]
```

Under "Customize" the user can grant a narrower pattern than requested:

```
App requested:  //net:**
User granted:   //net:*.google.com/**
```

The granted patterns are stored in the user's app policy database at
`//system:/etc/apps/<app-name>.policy`.

## Permission Pattern Syntax

| Pattern | Matches |
|---|---|
| `//net:**` | Any network path |
| `//net:*.google.com/**` | Any subdomain of google.com |
| `//net:192.168.1.1/**` | Specific IP |
| `//user:/documents/**` | All files under documents |
| `//user:/documents/*.pdf` | Only PDF files in documents |
| `//dev:cam:*` | Any camera device |
| `//dev:cam:0` | Front camera only |
| `//dev:mic:*` | Any microphone |
| `//dev:bt:**` | Any Bluetooth device |
| `//dev:gps:*` | Location hardware |

Wildcards:
- `*` — matches any single path segment (no `/`)
- `**` — matches any number of segments including `/`

## Built-in Permission Categories

For common cases the system defines named aliases that expand to patterns.
Apps can use the name in their manifest for readability:

| Name | Expands to | Meaning |
|---|---|---|
| `INTERNET` | `//net:**` | Full internet access |
| `LOCAL_NETWORK` | `//net:192.168.**` | LAN only |
| `BLUETOOTH` | `//dev:bt:**` | Any Bluetooth |
| `CAMERA` | `//dev:cam:**` | Any camera |
| `MICROPHONE` | `//dev:mic:**` | Any microphone |
| `LOCATION` | `//dev:gps:*` | GPS hardware |
| `DOWNLOADS` | `//user:/downloads/**` | Downloads folder |
| `DOCUMENTS` | `//user:/documents/**` | Documents folder |
| `REMOVABLE_STORAGE` | `//dev:usb:*/**` | USB drives |

Named aliases are resolved to path patterns before being stored. The kernel
only evaluates patterns — names do not exist at enforcement time.

## Enforcement in the VFS

The permission check runs in `vfs_open()`. Symlinks are always resolved to
their canonical target **before** the permission pattern is evaluated. This
prevents symlink traversal attacks where a permitted path contains a symlink
pointing outside the permitted subtree.

```c
int vfs_open(process_t *process, const char *path, int flags) {
    // 1. Resolve symlinks → canonical path (max 8 levels deep)
    char canonical[PATH_MAX];
    int result = vfs_resolve_path(path, canonical, 8);
    if (result == -ELOOP)  return -ELOOP;   // symlink cycle detected
    if (result < 0)        return result;

    // 2. Check canonical path against process permission list
    if (!permission_allows(process->granted_permissions, canonical)) {
        return -EACCES;
    }
    // 3. Resolve scheme → driver
    // 4. Check inode permissions (uid/rwx)
    // 5. Return fd
}
```

Both checks must pass. A process with `//user:/documents/**` permission still
cannot read a file owned by another user if the inode permissions deny it.

### Symlink Traversal Attack

A symlink traversal attack uses a symlink inside a permitted directory to
escape to a path outside the permitted subtree:

```
App permission: //user:/documents/**

Attacker creates:
  //user:/documents/passwords → symlink → //system:/etc/shadow

App opens //user:/documents/passwords
  → WITHOUT canonical resolution: path matches //user:/documents/** → allowed ← WRONG
  → WITH canonical resolution:    path resolves to //system:/etc/shadow
                                   does not match //user:/documents/** → -EACCES ← CORRECT
```

By resolving to the canonical path first, the permission check always operates
on the real destination regardless of how many symlink levels are involved.

### Hard Link Restriction

Hard links are more dangerous than symlinks because they are invisible — two
names point to the same inode with no indication in the path. The VFS prevents
hard link abuse at creation time:

```
// Creating a hard link is only permitted if the calling process
// has read permission for the target inode's canonical path.
// A process cannot create a hard link to a file it cannot already read.
```

This means an app with `//user:/documents/**` cannot create a hard link inside
documents pointing to an inode owned by another user or under `//system:`.

### Symlink Depth Limit

The VFS resolves symlinks up to a maximum depth of 8 levels. If resolution
exceeds this depth, `vfs_open()` returns `-ELOOP`. This prevents infinite loops
from circular symlinks:

```
//user:/documents/loop → symlink → //user:/documents/loop
→ -ELOOP after 8 attempts
```

## Permission Elevation (UAC-style)

When a process attempts to open a path not covered by its `.policy`, the kernel
does not immediately return `-EACCES`. Instead it fires a **permission elevation
event** which the system UI handles by showing a single dialog to the user.

### Grouping

Multiple missing permissions are grouped into one dialog shown at launch time,
not one dialog per `open()` call. This avoids the UAC-on-Vista problem where
users blindly approve dozens of prompts.

```
"editor" needs additional access:

  ● //user:/documents/**       Read and write
  ● //dev:disk:0/projects/**   Read and write
  ● //ram:/tmp/editor/**       Read and write

[Allow all]  [Deny]  [Customize...]
```

### Grant Duration

The user chooses how long the elevated permission lasts:

| Duration | Effect |
|---|---|
| **Permanent** | Added to the app's `.policy` file — survives reboot |
| **This session** | Held in memory — cleared when the process exits |
| **Once** | Applies to the single `open()` call that triggered the dialog |

### Abuse Detection

If a process requests more than a configurable threshold of undeclared
permissions within a short window (default: more than 10 distinct patterns
within 5 seconds), the kernel groups them into a single warning dialog:

```
"unknown-app" has requested access to 23 undeclared paths.
This may indicate malicious behavior.

[View full list]  [Deny all]
```

Well-written apps declare all needed permissions in their `.policy` at build
time. The elevation dialog is the exception, not the normal flow.

## Runtime Permission Queries

Apps can query and request permissions at runtime (not only at launch):

```c
/// Check if the current process has permission for a path.
/// Returns 1 if allowed, 0 if denied.
int has_permission(const char *path_pattern);

/// Request a permission at runtime. Triggers user approval UI if not already granted.
/// Returns 1 if granted, 0 if denied by user.
int request_permission(const char *path_pattern);
```

## Comparison with Other Systems

| System | Model | Granularity |
|---|---|---|
| Linux | Nothing by default; AppArmor/SELinux optional | Fine but complex |
| Android | Categorical permissions (`INTERNET`, `CAMERA`) | Coarse |
| iOS | Categorical with some path restrictions for files | Medium |
| macOS | Sandboxing with Seatbelt profiles (TinyScheme syntax) | Fine but opaque |
| **Bazzulto** | Path patterns covering all resource types uniformly | Fine and readable |

## Relationship to Other Features

- **Explicit-Scheme Paths** — schemes make it possible to express network,
  device, and file permissions in a single syntax. Without schemes, network
  access and file access require separate permission systems.
- **Path-Level Capabilities** — the per-process whitelist described in
  `FEATURES.md` is the kernel-side enforcement mechanism for this feature.
- **FD Capabilities** — once a file is open, FD rights control what the process
  can do with it. App permissions control whether it can open it at all.

## What This Is Not

This is not a sandboxing system that isolates processes from each other at the
memory level. It controls resource access through the VFS only. Memory
isolation is handled by the MMU and process address space separation.
