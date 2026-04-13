# Binary Permission Model

**Priority:** High — must be finalized before the ELF loader is extended and before
any third-party software distribution mechanism is designed.  
**Area:** `kernel/src/loader/`, `kernel/src/syscall/`, `kernel/src/fs/`,
`userspace/permissiond/`

**Related:** [App Permission Model.md](App%20Permission%20Model.md) — this document
extends that model to cover command-line binaries and unpackaged executables.

---

## Motivation

The App Permission Model defines how graphical applications declare and request
permissions via a `manifest.xml` bundle. That model covers UI apps well. It does
not cover:

- Command-line tools installed via the package manager (`curl`, `python`, `ffmpeg`)
- Binaries compiled locally (`cargo build`, `gcc`)
- Binaries downloaded and run directly without a package manager
- Scripts executed by an interpreter

These programs are the majority of software on a developer system. They must
participate in the permission model without requiring a full app bundle, while
preserving compatibility with existing Unix software that has no knowledge of
Bazzulto's permission system.

The goal is a model where:

1. Well-behaved software declares its own permissions and gets a clean
   first-run approval prompt.
2. Package-managed software is approved once at install time, never again at run time.
3. Unknown software degrades gracefully — it runs with inherited permissions
   and the user is informed, rather than being blocked.
4. No path through the model requires the user to type a root password or
   understand UID/GID.

---

## Approval Levels

Not all permission grants require the same level of user friction. The model
defines three approval levels based on the sensitivity of the requested
namespace. The level is determined by the **most sensitive pattern** in the
request — if any single pattern requires authentication, the entire prompt
requires authentication.

| Level | UX | When required |
|---|---|---|
| **Silent** | No prompt | Tier 2 policy exists; Tier 4 inheritance |
| **Consent** | Click prompt — no password | User-scoped namespaces, reversible |
| **Authenticated** | Click prompt + user password | System-affecting, device-level, hard to reverse |
| **Impossible** | Kernel hard-rejects | System-only namespaces — never user-grantable |

### Namespace sensitivity table

| Namespace | Level | Rationale |
|---|---|---|
| `//user:/home/<user>/**` | Consent | User's own data |
| `//user:/documents/**` | Consent | User's own data |
| `//user:/downloads/**` | Consent | User's own data |
| `//ram:/tmp/**` | Consent | Ephemeral, low risk |
| `//net:**` | Consent | User decides internet access |
| `//net:<specific-host>/**` | Consent | Narrower than full internet |
| `//dev:cam:**` | Consent | Camera access |
| `//dev:mic:**` | Consent | Microphone access |
| `//dev:bt:**` | Consent | Bluetooth |
| `//dev:gps:**` | Consent | Location |
| `//dev:usb:**` | Authenticated | Removable storage / hardware |
| `//dev:raw:**` | Authenticated | Raw device access |
| `//sys:driver/**` | Authenticated | Load/unload kernel drivers |
| `//sys:mount/**` | Authenticated | Mount filesystems |
| `//sys:pkg/**` | Authenticated | Install/uninstall software |
| `//sys:net:config/**` | Authenticated | Change network configuration |
| `//sys:kernel/**` | Authenticated | Kernel parameters |
| `//sys:policy:write/**` | **Impossible** | Modify other processes' policies |
| `//sys:perm:grant/**` | **Impossible** | Grant permissions to other processes |
| `//system:/bin/**` (write) | **Impossible** | Write system binaries |
| `//system:/sbin/**` (write) | **Impossible** | Write system binaries |

### Authenticated prompt example

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Privileged Access Request
  Binary: /home/arael/tools/driver-installer

  This program is requesting elevated access to:
    ● //sys:driver/**    Load kernel drivers

  This operation requires your password.

  Password: ________________

  [Allow]  [Deny]  [Customize...]
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

The password is the **user's login password**, not a separate root password.
There is no root password. Authentication proves it is the account owner
approving the action, not a third party or an automated script.

`permissiond` verifies the password against the user credential store and only
proceeds if verification succeeds. Failed authentication returns `EACCES` to
the blocked process.

---

## The Four Tiers

Every `exec()` call goes through a four-tier dispatch. The first matching tier
wins.

```
┌──────────────────────────────────────────────────────────────────┐
│ exec() permission dispatch                                        │
├─────────┬────────────────────────────────────────────────────────┤
│ Tier 1  │ System binary   — //system:/bin/** or //system:/sbin/** │
│         │ → Full trust. No prompt. No inheritance check.          │
├─────────┼────────────────────────────────────────────────────────┤
│ Tier 2  │ Policy exists   — //sys:policy:{sha256} or               │
│         │                   //sys:policy:{sha256}:{uid}            │
│         │ → Load policy. Execute. No prompt.                       │
├─────────┼────────────────────────────────────────────────────────┤
│ Tier 3  │ ELF section present — .bazzulto_permissions             │
│         │ → Parse declared permissions. Show approval prompt.      │
│         │ → On accept: write policy, execute.                      │
│         │ → On deny: abort exec, return EPERM.                     │
├─────────┼────────────────────────────────────────────────────────┤
│ Tier 4  │ No section, no policy (unknown binary)                  │
│         │ → Inherit parent's granted_permissions.                  │
│         │ → Emit warning to stderr.                                │
│         │ → Execute with inherited set.                            │
└─────────┴────────────────────────────────────────────────────────┘
```

---

## Tier 1 — System Binaries

Any executable whose canonical path begins with `//system:/bin/` or
`//system:/sbin/` and carries a valid **Ed25519 signature** from the Bazzulto
system key is implicitly trusted. These are the OS core utilities distributed
and verified by the system. They receive the full permission set.

### Signature verification

The system ELF signing key (Ed25519) is embedded in the kernel image at build
time. At exec() time, before any page mapping:

1. The kernel reads the `.baz_sig` ELF note section (64-byte Ed25519 signature
   over the SHA-256 of the binary's content).
2. Verifies the signature against the embedded public key.
3. If the signature is valid → Tier 1. If invalid or absent → fall through to
   Tier 2/3/4 dispatch.

Path alone is not sufficient — a binary placed in `//system:/bin/` without a
valid signature is not Tier 1. This prevents an attacker who gains write access
to the system partition from silently elevating their binary.

**No prompt is shown. No policy store is consulted.**

System binaries include: `shell`, `ls`, `cat`, `cp`, `mv`, `rm`, `mkdir`,
`chmod`, `chown`, `mount`, `ifconfig`, `baz` (package manager), `init`,
`permissiond`.

---

## Tier 2 — Policy File

When a policy file exists for a binary, the approval was already given (either
at package install time, or during a previous Tier 3 prompt). The kernel loads
the policy and executes without user interaction.

### Policy storage — kernel-owned virtual namespace

Policies are stored in the `//sys:policy:` virtual filesystem, implemented
inside the kernel's policy module (`kernel/src/permission/mod.rs`). This
namespace is **not backed by a regular filesystem directory** accessible via
`open()`.

```
//sys:policy:{merkle_root}            — one entry per binary, keyed by Merkle root
//sys:policy:{merkle_root}:{uid}      — per-user entry (Consent-level grants)
```

The key is the **Merkle root** over the binary and all its shared library
dependencies (resolved at exec() time via the dynamic linker's load list):

```
MerkleRoot = Hash(
    Hash(binary_bytes),
    Hash(libfoo.so_bytes),
    Hash(libbar.so_bytes),
    ...  sorted by canonical path to be deterministic
)
```

If any dependency changes (library update, supply chain injection), the Merkle
root changes → the old policy entry no longer matches → Tier 3 prompt on next
run. This closes the dependency confusion attack: a compromised `.so` invalidates
the policy even when the binary itself is unchanged.

For static binaries (no shared libraries), the Merkle root is the SHA-256 of
the binary's content.

#### Read access (userspace)

`open("//sys:policy:{sha256}", O_RDONLY)` is permitted. Userspace tools
(`baz list`, permission inspector) can read the current policy set for any
binary. The returned content is plain text, one pattern per line.

#### Write access (blocked at VFS)

Any `open(..., O_WRONLY | O_RDWR | O_CREAT | O_TRUNC)` on any path under
`//sys:policy:` returns `EACCES` unconditionally, regardless of UID, euid, or
capability. The VFS handler for this namespace hard-codes this refusal — there
is no privilege level that bypasses it.

#### Write access (syscall only)

The only write paths are the `grant_permissions()` and `revoke_permissions()`
syscalls (see §permissiond). These call into the kernel policy module directly,
bypassing the VFS open path. The kernel module then updates the in-memory policy
store and flushes to the persistent backing store.

#### Persistent backing store

The kernel writes policy data to a binary blob at a well-known inode on the
root filesystem. This inode carries an internal `INODE_KERNEL_INTERNAL` flag:
the VFS layer rejects any userspace `open()` on it (ENOENT — it appears not
to exist). The kernel writes to it directly via an internal function, never
through the userspace-accessible VFS path.

This means:
- **Persistent**: survives reboots. Travels with the disk.
- **Portable**: a disk moved to another machine carries all policy entries.
- **Immutable from userspace**: no shell command, no UID 0, no `dd`, no debugger
  can modify the backing store. The only write surface is `grant_permissions()`.

#### TOCTOU mitigation

The SHA-256 hash is computed over the **open file descriptor** at exec() time.
The kernel opens the ELF binary → hashes its bytes through the open fd →
performs policy lookup → parses and loads ELF from the same fd. Swapping the
binary on disk after exec() starts has no effect because all operations use
the already-opened inode reference, not the path.

### Policy file format (as returned by read())

Plain text, one path pattern per line. Lines beginning with `#` are comments.

```
# curl 8.7.1 — installed 2026-03-15
//net:**
//user:/downloads/**
//ram:/tmp/curl/**
```

### Policy scoping — per-user vs system-wide

| Grant type | Storage key | Who can write | Who it applies to |
|---|---|---|---|
| Consent-level | `//sys:policy:{sha256}:{uid}` | permissiond, after user click | Only that UID |
| Authenticated-level | `//sys:policy:{sha256}` | permissiond, after admin password | All users on this machine |

A Consent-level grant by user A (UID 1000) is stored under a different key than
user B (UID 1001). User B gets their own Tier 3 prompt on first run.

An Authenticated-level grant has no UID suffix — it applies to all users.
Only members of the `admin` group can produce Authenticated-level entries
(see §User authority model).

### User authority model

| Approval level | Who can approve | Grant scope |
|---|---|---|
| Consent (click) | Any logged-in user | Per-user (`{sha256}:{uid}`) |
| Authenticated (password) | Admin group members only | System-wide (`{sha256}`) |

`permissiond` checks the requesting user's group membership against `/etc/group`
before presenting an Authenticated prompt. If the user is not in the `admin`
group, the prompt is suppressed and `EPERM` is returned immediately — no
password dialog is shown.

This prevents a non-admin user from escalating a binary to system-wide policy,
even if they know the password. Admin group membership is the gate, not
knowledge of a password.

### Revoking permissions

`revoke_permissions(sha256, scope)` is the mirror of `grant_permissions()`,
also callable only by permissiond:

| Scope | Effect |
|---|---|
| `User(uid)` | Remove `{sha256}:{uid}` entry from policy store |
| `System` | Remove `{sha256}` (system-wide) entry |
| `All` | Remove all entries for this sha256 across all users and system |

Revocation affects future `exec()` calls only. Running processes retain their
in-memory `granted_permissions` until exit — the same semantics as Unix file
permission changes not affecting already-open descriptors.

Use cases: `baz uninstall <pkg>` calls `revoke_permissions(sha256, All)`; a
user removing a previously-approved app; security incident response.

---

## Tier 3 — ELF Section Declaration

A binary that declares its own permissions embeds them in an ELF section named
`.bazzulto_permissions`. The section contains a null-terminated list of
null-terminated UTF-8 path pattern strings.

### ELF section format

```
Section name:  .bazzulto_permissions
Section type:  SHT_NOTE (0x7)
Flags:         SHF_ALLOC (readable in the loaded image)
Contents:      sequence of null-terminated UTF-8 strings, terminated by
               an empty string (double null byte)
```

Example contents (hex bytes):
```
2f 2f 6e 65 74 3a 2a 2a 00           //net:**\0
2f 2f 75 73 65 72 3a 2f 64 6f ...00  //user:/downloads/**\0
00                                    terminator
```

### Prompt flow

When the ELF loader finds a `.bazzulto_permissions` section and no existing
policy, it:

1. Sends a `PERM_REQUEST` event to `permissiond` via a dedicated kernel IPC
   channel, including:
   - Binary path
   - SHA-256 hash of the binary
   - List of declared patterns

2. Blocks the exec() syscall — the calling process is suspended.

3. `permissiond` presents the approval dialog to the user (via the system UI or
   terminal fallback, depending on context).

4. The user responds: Allow All, Deny, or Customize.

5. `permissiond` sends the decision back to the kernel via `grant_permissions()`.

6. The kernel either:
   - Writes the approved policy to `~/.config/apps/<hash>.policy` and resumes
     exec() with those permissions, or
   - Returns `EPERM` to the calling process if denied.

### Terminal fallback prompt

When no display server is running (early boot, SSH session, headless system),
`permissiond` falls back to an in-terminal prompt:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Permission Request
  Binary: /home/arael/tools/my-downloader
  Hash:   a3f2c1...

  This program is requesting access to:
    [1] //net:**              Internet (any host)
    [2] //user:/downloads/**  Your downloads folder
    [3] //ram:/tmp/**         Temporary files

  [A] Allow all   [D] Deny   [C] Customize   [1/2/3] Toggle
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

The shell calling exec() is blocked while this prompt is shown. After the user
responds, the shell resumes with either the process running or an error message.

### Prompt grouping — the no-UAC-fatigue rule

A single binary can only trigger one Tier 3 prompt per policy hash. All
declared permissions are shown together in one dialog, not one dialog per
permission. Once a policy exists, no further prompts occur for that binary
regardless of how many times it is executed.

### Grant duration

The user selects how long the approval applies:

| Duration | Effect |
|---|---|
| **Permanent** | Written to `~/.config/apps/<hash>.policy`. Survives reboot. |
| **This session** | Held in memory in the current process tree. Cleared when the shell session exits. |
| **Once** | Applied to this single exec() only. Not written to disk. |

---

## Tier 4 — Unknown Binary (Inheritance with Warning)

A binary with no `.bazzulto_permissions` section and no existing policy file is
an unknown binary. This includes:

- Binaries freshly compiled with a toolchain that does not emit the section
- Binaries downloaded and run without going through the package manager
- Scripts executed via an interpreter that itself is a Tier 1 binary

The kernel does not block these. Instead:

1. The child process inherits `granted_permissions` from the parent verbatim.
2. The kernel writes a warning line to the process's stderr before the binary
   starts executing:

```
[bazzulto] warning: /path/to/binary has no permission declaration.
           Running with inherited permissions from parent process.
           To suppress: add a .bazzulto_permissions ELF section.
```

3. The process runs normally with the inherited permission set.

### Why not block

Blocking unknown binaries would make it impossible to run freshly compiled
code, scripts, or any Unix software without modifying its build system. This is
unacceptable for a developer-facing OS. The warning-and-inherit model preserves
compatibility while making the security posture visible.

Over time, ecosystem pressure — developer tools, linters, and package standards
— will drive adoption of the ELF section for software that cares about
expressing its intent.

### The terminal as a permission scope boundary

The practical effect of Tier 4 is that the terminal session's permission set
acts as the outer boundary for all unknown software run within it. A terminal
opened by a regular user has permissions roughly equivalent to:

```
//user:/home/<username>/**   (user's home directory)
//user:/tmp/**               (temporary files)
//net:**                     (network — approved when terminal was first launched)
```

A Tier 4 binary run from that terminal cannot exceed those permissions, even if
it tries — the VFS `vfs_open()` check enforces the limit regardless of what the
binary attempts.

An attacker who plants a binary in a writable directory and tricks the user into
running it gains at most the terminal's permission set, not system-wide access.

---

## Package Manager Integration (Tier 2 shortcut)

The package manager (`baz`) reads the `.bazzulto_permissions` section from each
ELF in the package at install time. It presents the permissions to the user
**once**, at install time, not at first run:

```
$ baz install curl

Installing curl 8.7.1
  Permissions requested:
    ● //net:**              Internet access
    ● //ram:/tmp/curl/**    Temporary files

[Install]  [Cancel]
```

If the user approves, `baz` calls `grant_permissions(sha256, patterns)` and
the install completes. The kernel writes the entry to `//sys:policy:{sha256}`.
All subsequent executions of `curl` hit Tier 2 — no runtime prompt, no friction.

If the package contains a binary without a `.bazzulto_permissions` section, the
package manager warns at install time:

```
  Warning: /bin/legacy-tool has no permission declaration.
  It will run with inherited terminal permissions.
```

The install still proceeds. The binary will hit Tier 4 at runtime.

---

## The `permissiond` Daemon

`permissiond` is a Tier 1 userspace daemon that mediates all permission prompts
between the kernel and the user. It holds the following system permissions:

```
//sys:perm:grant            → can call grant_permissions() and revoke_permissions()
//sys:display:**            → access to display server (for UI prompts)
//dev:tty:**                → access to terminal (for terminal prompts)
```

No other process can call `grant_permissions()`. The kernel enforces this.

### Kernel ↔ permissiond protocol

```
Kernel sends (PERM_REQUEST event):
  struct PermRequest {
      pid:          u32,       // blocked process
      binary_path:  String,
      binary_hash:  [u8; 32],  // SHA-256
      patterns:     Vec<String>,
  }

permissiond responds (grant_permissions syscall):
  struct PermGrant {
      pid:       u32,
      decision:  Allow(Vec<String>) | Deny,
      duration:  Permanent | Session | Once,
  }
```

The kernel IPC channel is a dedicated message queue — not a Unix socket, not a
pipe — to prevent TOCTOU attacks where a compromised process intercepts
permission messages.

---

## Customization — Narrowing Declared Permissions

The "Customize" option in the approval dialog lets the user grant a narrower
pattern than the one declared:

```
Binary declared:  //net:**
User customized:  //net:*.my-trusted-host.com/**
```

The binary still calls `open("//net:malicious.com/payload")` — the kernel
returns `EACCES`.

This is the same mechanism described in the App Permission Model. Binary
permissions are a subset of the same system — the kernel enforces patterns
uniformly regardless of whether the process is a UI app or a CLI binary.

---

## Interpreter and Script Handling

When a script is executed via a shebang (`#!/usr/bin/python3`), the kernel
exec()s the interpreter, not the script. The permission tier is determined by
the **interpreter's** ELF, not the script file.

| Interpreter | Tier | Permission source |
|---|---|---|
| `/usr/bin/python3` | Tier 1 (system binary) | Full trust |
| `~/bin/python3` with ELF section | Tier 3 | Declared section + prompt |
| `~/bin/python3` without section | Tier 4 | Inherited from parent |

A script file itself has no permission declaration — it is data fed to the
interpreter. If fine-grained per-script permissions are needed in the future, a
comment-based declaration syntax could be introduced as a convention for
interpreters that opt in.

---

## ELF Toolchain Support

For a binary to participate in Tier 3, its build system must emit the
`.bazzulto_permissions` section. The canonical way to do this:

### C / C++ (linker script or objcopy)

```c
// permissions.c — compile and link into the binary
__attribute__((section(".bazzulto_permissions")))
static const char bazzulto_permissions[] =
    "//net:**\0"
    "//user:/downloads/**\0"
    "\0";  // terminator
```

### Rust (build.rs or link section attribute)

```rust
#[link_section = ".bazzulto_permissions"]
#[used]
static PERMISSIONS: &[u8] =
    b"//net:**\0//user:/downloads/**\0\0";
```

### baz package format

Packages built with the Bazzulto SDK declare permissions in `manifest.toml`:

```toml
[permissions]
patterns = [
    "//net:**",
    "//user:/downloads/**",
    "//ram:/tmp/my-tool/**",
]
```

The build toolchain reads this and emits the ELF section automatically. The
same patterns appear in the `baz install` prompt and in the runtime policy —
there is exactly one source of truth.

---

## VFS Enforcement — Anti-Enumeration

When `vfs_open()` fails a permission check, the kernel returns `EACCES`
regardless of whether the resource exists at that path:

```
vfs_open(process, path):
  1. Resolve canonical path
  2. permission_allows(process.granted_permissions, canonical)?
     → No: return EACCES immediately (do not check inode existence)
  3. inode_exists(canonical)?
     → No: return ENOENT
  4. inode_permission_check(inode, process.uid, process.gid)
     → No: return EACCES
  5. Open
```

Step 2 short-circuits before any inode lookup. An attacker who probes
`//sys:secret/passwords` with no `//sys:**` grant receives `EACCES` whether
the resource exists or not. The error code reveals nothing about existence.

---

## Runtime Permission Elevation

The permission model supports **minimal initial privilege** — a binary can
start with a narrow declared set and request additional access at the moment
it is actually needed.

### sys_request_cap

```rust
// sys_request_cap(patterns: &[&str], reason: &str) -> Result<GrantedSet, i64>
```

When called, the kernel suspends the calling thread and forwards a
`PERM_REQUEST` event to `permissiond` (same protocol as Tier 3 prompt). The
user sees a targeted dialog explaining why the app needs the new permission
at this moment. On approval, the kernel adds the granted patterns to
`process.granted_permissions` and resumes the thread.

The `reason` string is shown verbatim in the prompt. It is the app's
responsibility to provide a clear, user-readable explanation.

**Rate limiting**: if a process calls `sys_request_cap` for the same pattern
more than 3 times within 60 seconds after the user denied it, the kernel
returns `EACCES` without forwarding to permissiond. Repeated prompting after
denial is treated as a misbehaving app.

### Powerbox

The Powerbox mechanism allows the user to grant a **surgical, single-resource
permission** by choosing a resource through a system-provided picker:

```rust
// sys_powerbox_open(mode: OpenMode) -> Result<Fd, i64>
// Opens the system file picker. The user selects a file.
// The kernel returns an open fd for the selected file, regardless of
// whether the calling process has //user:/home/** permission.
// No path pattern is added to granted_permissions — only this specific fd
// is granted.
```

The Powerbox model:
- The app never learns the path of the selected file (it receives only the fd).
- The kernel validates the user's selection against the inode — the process
  gets access to exactly the chosen resource, nothing else.
- Suitable for "open file" / "save to" dialogs without granting broad
  filesystem access.

---

## Interpreter Self-Restriction

A Tier 1 interpreter (e.g. `python3` in `//system:/bin/`) starts with full
trust. Before executing a user-provided script (third-party code), the
interpreter should call `sys_restrict_self` to drop its own privileges:

```rust
// sys_restrict_self(patterns: &[&str]) -> Result<(), i64>
// Replaces granted_permissions with the given set (which must be a subset
// of the current set). Irreversible for the lifetime of this process.
```

This allows a Tier 1 Python interpreter to run user scripts with only
`//user:/home/**` access instead of full system trust. The interpreter
calls `sys_restrict_self(["//user:/home/**"])` before `exec`-ing the script's
`__main__`.

Interpreters that do not call `sys_restrict_self` run user scripts with the
interpreter's full permission set — this is a known limitation for Tier 1
interpreters that have not been updated. The warning log emitted by Tier 4
inheritance is analogous and serves as a signal to maintainers.

---

## IPC File Descriptor Re-Validation

When a file descriptor is passed between processes via `SCM_RIGHTS`
(sendmsg/recvmsg), the kernel re-validates the receiving process's permission
set against the resource underlying the fd:

```
recvmsg with SCM_RIGHTS:
  for each received fd:
    1. Determine the canonical path / resource of the underlying inode.
    2. Check permission_allows(receiver.granted_permissions, canonical).
    3. If no: close the received fd and replace with -EACCES in the cmsg.
```

A process with high trust cannot silently hand a sensitive fd to a low-trust
process. The receiving process must have the appropriate permission grant for
the resource, not just for the socket it used to receive the fd.

---

## Security Properties

### What this model prevents

| Attack | Prevention |
|---|---|
| Malicious binary reads /etc/shadow | System path not in any user-granted permission; EACCES at vfs_open |
| Malicious binary exfiltrates home dir | Network access requires //net:** permission; prompt shown on first run |
| Dependency confusion / supply chain injection | Merkle root covers binary + all .so deps; any library change invalidates policy |
| Replaced Tier 1 binary (unsigned) | Ed25519 signature required; unsigned binary falls through to Tier 2/3/4 |
| Privilege escalation via symlink | Canonical path resolved before permission check |
| Script injection into interpreter | sys_restrict_self drops interpreter to script-safe permission set |
| File existence enumeration | EACCES returned regardless of resource existence; can't probe namespace structure |
| fd injection via SCM_RIGHTS | Kernel re-validates receiver's granted_permissions for each received fd |
| UAC fatigue (repeated prompts) | Rate limiting: 3 denials in 60s → EACCES without further prompting |

### What this model does NOT prevent

| Scenario | Note |
|---|---|
| Malicious code within declared permissions | If the user approved //net:**, a malicious binary can reach any host on the network. Permissions constrain the surface, not intent. |
| Tier 4 binaries running with terminal scope | A compromised binary inherits the terminal's permissions. Mitigation: users should not run untrusted binaries from a terminal with broad permissions. |
| permissiond compromise | If permissiond is compromised, all permission grants are compromised. permissiond must be minimal, audited, and isolated. |
| Interpreter that ignores sys_restrict_self | Tier 1 interpreters that do not call sys_restrict_self run user scripts with full trust. Requires interpreter cooperation. |

### Future — TPM binding

On hardware with a Trusted Platform Module, per-user policy entries can be
sealed to the device's TPM, preventing offline modification (mounting the disk
on another machine). A TOFU (Trust On First Use) record stored in the TPM
ensures that even if the filesystem is copied, the policy keys are not
transferable.

QEMU `virt` does not have a TPM. This is deferred until real hardware support
is added.

---

## Relationship to the UID/GID Model

This model operates **above** the UID/GID layer. Both checks must pass to open
a resource:

```
vfs_open(process, path):
  1. Resolve canonical path (follow symlinks, check depth)
  2. permission_allows(process.granted_permissions, canonical) → EACCES if no
  3. inode_permission_check(inode, process.uid, process.gid) → EACCES if no
  4. Open
```

A process with `//user:/documents/**` permission but the wrong UID for a file
still gets EACCES. A process with the right UID but without the path permission
also gets EACCES. Both gates must be open.

In practice, for a single-user system, UID/GID distinctions primarily matter
for separating system processes from user processes. The path permission model
provides the meaningful per-app granularity that UID alone cannot express.

---

## Relationship to the Existing App Permission Model

This document does not replace [App Permission Model.md](App%20Permission%20Model.md).
It extends it with the binary tier dispatch. The core enforcement mechanism
(path patterns in `vfs_open()`, `permissiond`, `.policy` files, grant duration)
is the same for both UI apps and CLI binaries. The only difference is how the
permission set is declared:

| App type | Declaration | Approval point |
|---|---|---|
| UI app (bundle) | `manifest.xml` in bundle | First launch |
| CLI binary (Tier 3) | `.bazzulto_permissions` ELF section | First execution |
| Package-managed binary | ELF section, read by `baz` | Install time |
| System binary | Implicit (path-based) | Never |
| Unknown binary | None | Never (inherited) |

---

## Implementation Order

This feature has kernel and userspace components that must be implemented in
sequence:

1. **ELF loader reads `.bazzulto_permissions` section** — `kernel/src/loader/mod.rs`.
   Parse the null-terminated string list. Store in a temporary buffer before
   exec() completes.

2. **`granted_permissions: Vec<PathPattern>` in Process** — `kernel/src/process/mod.rs`.
   Inherited by fork(); replaced by exec() based on tier dispatch.

3. **`vfs_open()` permission check** — `kernel/src/fs/vfs.rs` and `kernel/src/syscall/mod.rs`.
   After canonical path resolution, before inode check.

4. **Tier 1 check** — path prefix match against `//system:/bin/**` and
   `//system:/sbin/**` at exec() time.

5. **Policy store load (Tier 2)** — query `//sys:policy:{sha256}` (system-wide)
   and `//sys:policy:{sha256}:{uid}` (per-user) at exec() time.

6. **`permissiond` daemon and IPC channel** — userspace daemon plus kernel
   message queue for permission events.

7. **Tier 3 prompt flow** — kernel sends `PERM_REQUEST`, blocks process,
   resumes or aborts on `permissiond` response.

8. **Tier 4 inheritance with warning** — copy `granted_permissions` from parent,
   emit warning to stderr.

9. **Package manager integration** — `baz install` reads ELF section, shows
   prompt, calls `grant_permissions(merkle_root, patterns)` on approval.

10. **Ed25519 Tier 1 signature verification** — embed public key in kernel
    image; verify `.baz_sig` ELF note at exec() for `//system:/bin/**` paths.

11. **Merkle root computation** — at exec(), resolve `.so` dependencies and
    compute Merkle root over binary + all libraries. Use as policy store key.

12. **`sys_request_cap`** — runtime permission elevation syscall + rate
    limiting (3 denials / 60s per pattern per process).

13. **Powerbox — `sys_powerbox_open`** — system file picker returning fd
    without exposing path to calling process.

14. **`sys_restrict_self`** — irreversible privilege reduction syscall;
    update Tier 1 interpreters to call it before executing user scripts.

15. **IPC fd re-validation** — in `recvmsg` SCM_RIGHTS handling, validate
    receiver's `granted_permissions` for each received fd.

16. **Toolchain helpers** — Rust and C snippets/macros for emitting the ELF
    section, included in the SDK.
