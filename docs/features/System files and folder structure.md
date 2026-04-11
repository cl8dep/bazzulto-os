# System Files and Folder Structure

Bazzulto uses an explicit-scheme path model. Every resource in the system —
files, devices, network endpoints, processes, pipes — is addressed by a path
of the form:

```
//scheme:authority/path
```

The scheme identifies the **type** of resource. The kernel guarantees the
contract of each scheme — `//ram:` is always volatile memory, `//system:` is
always read-only. Unlike Unix mount points, these contracts cannot be changed
by remounting.

---

## Path Format

```
//scheme:authority/path/to/resource
  ^^^^^^  ^^^^^^^^^  ^^^^^^^^^^^^^^^^
  type    device or  resource location
          host id    within that device
```

The authority is optional for schemes that have only one instance:

```
//ram:/tmp/file.txt       → authority omitted (only one RAM filesystem)
//system:/bin/shell       → authority omitted (only one system partition)
//dev:disk:0/home/...     → authority required (multiple disks possible)
//dev:bt:0/               → authority required (multiple BT adapters possible)
```

---

## System Schemes

### `//system:` — OS partition (read-only)

The operating system itself. The kernel enforces read-only at the scheme level —
no process can open a path under `//system:` with write access regardless of
inode permissions.

```
//system:/bin/            → system binaries (shell, ls, cat, ...)
//system:/lib/            → system static libraries
//system:/etc/            → system-wide configuration
//system:/etc/apps/       → per-app permission policies
//system:/boot/           → kernel ELF, boot configuration
```

### `//home:` — All user home directories

The root of all user accounts. Equivalent to `/home/` on Unix.

```
//home:/arael/            → home directory of user "arael"
//home:/arael/documents/  → documents
//home:/arael/downloads/  → downloads
//home:/maria/            → home directory of user "maria"
```

### `//user:` — Current user's home (`~`)

Alias for the home directory of the currently logged-in user. The VFS expands
`//user:` to the matching entry under `//home:` at resolution time.

```
//user:/documents/        → ~/documents/
//user:/downloads/        → ~/downloads/
//user:/config/app.conf   → ~/.config/app.conf  (Unix equivalent)
```

### `//ram:` — Volatile in-memory filesystem

Always RAM. Always lost on reboot. Cannot be remounted to disk.

```
//ram:/tmp/               → temporary files
//ram:/run/               → runtime state, named pipes, sockets
//ram:/run/<app>/         → per-app IPC endpoints
//ram:/dev/               → virtual devices (see below)
```

### `//dev:` — Hardware and virtual devices

All physical and virtual devices live under `//dev:` with an explicit type and
index.

```
//dev:<type>:<index>/
```

See the full device table below.

### `//proc:<pid>/` — Process filesystem

Each running process is accessible as a directory. The PID is the authority.

```
//proc:1/                 → process 1 (init)
//proc:1/name             → process name (plain text)
//proc:1/memory           → memory usage stats
//proc:1/fds/             → open file descriptors
//proc:1/fds/0            → fd 0 (stdin)
//proc:1/permissions      → granted permission patterns
//proc:self/              → alias for the calling process
```

Readable by the owning process and by root. `/memory` and `/fds/` are private.
`/name` is public.

### `//net:<host>/` — Network resources

```
//net:192.168.1.10/share/file.txt
//net:files.example.com/public/doc.pdf
//net:*.internal/config
```

Subject to app permission patterns (see App Permission Model).

### `//pipe:` — Named pipes (IPC)

```
//pipe:/logger            → named pipe for the logger service
//pipe:/audio/sink        → audio output pipe
```

---

## Device Table (`//dev:`)

### Disks (`disk`)

```
//dev:disk:0/             → first disk (boot order)
//dev:disk:1/             → second disk
//dev:disk:main/          → disk with label "main"
//dev:disk:sda/           → disk by device name
```

**Multiple disks:** the authority is the index (0-based, enumerated at boot) or
the disk label. Labels are set when formatting with `mkfs.bazzulto`.

### USB storage (`usb`)

```
//dev:usb:0/              → first USB storage device
//dev:usb:1/              → second USB storage device
```

Removable. The kernel fires a device-attach event when a USB drive is inserted.
The index is assigned in plug-in order and released when the device is removed.

### Bluetooth (`bt`)

```
//dev:bt:0                → first Bluetooth adapter
//dev:bt:1                → second Bluetooth adapter (e.g. USB dongle + built-in)
//dev:bt:0/paired/        → paired devices visible as subdirectory
//dev:bt:0/paired/speaker → a specific paired device
```

**Multiple adapters:** index 0 is the primary adapter (built-in). Additional
adapters (USB dongles) get the next available index. If adapter 0 is removed,
indices do not shift — the index is fixed at enumeration time until reboot.

### Cameras (`cam`)

```
//dev:cam:0               → first camera (front-facing on laptops)
//dev:cam:1               → second camera (rear-facing or external)
```

### Microphones (`mic`)

```
//dev:mic:0               → default microphone
//dev:mic:1               → secondary microphone
```

### Audio output (`audio`)

```
//dev:audio:0             → default audio output (speakers/headphones)
//dev:audio:1             → secondary output (HDMI, USB DAC)
```

### GPS / Location (`gps`)

```
//dev:gps:0               → primary location hardware
```

### Display (`display`)

```
//dev:display:0           → primary display
//dev:display:1           → secondary display
```

### Keyboard and pointer (`kbd`, `ptr`)

```
//dev:kbd:0               → first keyboard
//dev:ptr:0               → first pointer device (mouse, trackpad)
```

### Virtual devices (under `//ram:/dev/`)

These are not physical hardware — they are always available and live in RAM:

```
//ram:/dev/null           → discards all writes, reads return EOF
//ram:/dev/zero           → reads return infinite zero bytes
//ram:/dev/random         → cryptographic random bytes
//ram:/dev/stdin          → alias for fd 0 of current process
//ram:/dev/stdout         → alias for fd 1 of current process
//ram:/dev/stderr         → alias for fd 2 of current process
```

---

## Full Scheme Reference

| Scheme | R/W | Authority | Description |
|---|---|---|---|
| `//system:` | R only | none | OS partition |
| `//home:` | R/W | username | All user home directories |
| `//user:` | R/W | none | Current user home (`~`) |
| `//ram:` | R/W | none | Volatile memory filesystem |
| `//dev:disk:N/` | R/W | index or label | Persistent disk |
| `//dev:usb:N/` | R/W | index | USB storage |
| `//dev:bt:N` | R/W | index | Bluetooth adapter |
| `//dev:cam:N` | R only | index | Camera |
| `//dev:mic:N` | R only | index | Microphone |
| `//dev:audio:N` | R/W | index | Audio device |
| `//dev:gps:N` | R only | index | Location hardware |
| `//dev:display:N` | R/W | index | Display |
| `//dev:kbd:N` | R only | index | Keyboard |
| `//dev:ptr:N` | R only | index | Pointer device |
| `//proc:<pid>/` | R only* | PID | Process information |
| `//net:<host>/` | R/W | hostname or IP | Network resource |
| `//pipe:<name>` | R/W | name | Named pipe / IPC |

*`//proc:self/` is writable for select fields (e.g. process name).

---

## Code Examples for App Developers

All examples use the standard C `open`/`read`/`write`/`close` API. The path
is the only thing that changes — the rest of the API is identical to POSIX.

### Reading a file from the user's documents

```c
#include <libc/stdio.h>
#include <libc/stdlib.h>
#include <library/systemcall.h>

void print_document(const char *filename) {
    char path[256];
    snprintf(path, sizeof(path), "//user:/documents/%s", filename);

    int fd = open(path);
    if (fd < 0) {
        fprintf(2, "error: cannot open %s\n", path);
        return;
    }

    char buf[512];
    int64_t bytes;
    while ((bytes = read(fd, buf, sizeof(buf) - 1)) > 0) {
        buf[bytes] = '\0';
        write(1, buf, bytes);
    }
    close(fd);
}
```

### Writing a temporary file

```c
// Temporary files go to //ram:/tmp/ — automatically cleaned up on reboot.
// No explicit cleanup needed unless you want to free RAM immediately.

int write_temp(const char *name, const char *content, size_t length) {
    char path[256];
    snprintf(path, sizeof(path), "//ram:/tmp/%s", name);

    int fd = open(path);   // kernel creates the file if it does not exist
    if (fd < 0) return -1;

    write(fd, content, length);
    close(fd);
    return 0;
}
```

### Listing files on a USB drive

```c
void list_usb(int usb_index) {
    char name[256];
    int64_t size;

    for (int i = 0; ; i++) {
        char prefix[64];
        snprintf(prefix, sizeof(prefix), "//dev:usb:%d", usb_index);

        size = list(i, name, sizeof(name));
        if (size < 0) break;   // no more files

        printf("%s/%s  (%lld bytes)\n", prefix, name, size);
    }
}
```

### Reading from a named pipe (IPC between two apps)

```c
// Producer — writes events to a named pipe
void producer(void) {
    int fd = open("//pipe:/events/keyboard");
    write(fd, "key:A\n", 6);
    close(fd);
}

// Consumer — reads events from the same pipe
void consumer(void) {
    int fd = open("//pipe:/events/keyboard");
    char buf[64];
    int64_t bytes = read(fd, buf, sizeof(buf) - 1);
    if (bytes > 0) {
        buf[bytes] = '\0';
        printf("event: %s\n", buf);
    }
    close(fd);
}
```

### Checking process information

```c
// Read the name of process 42
void print_process_name(int pid) {
    char path[64];
    snprintf(path, sizeof(path), "//proc:%d/name", pid);

    int fd = open(path);
    if (fd < 0) {
        printf("process %d not found\n", pid);
        return;
    }

    char name[128];
    int64_t bytes = read(fd, name, sizeof(name) - 1);
    name[bytes] = '\0';
    printf("process %d: %s\n", pid, name);
    close(fd);
}
```

### Declaring permissions in your app

```c
// Declare at the top of your main.c.
// The linker embeds this in the .bz_permissions ELF section.
// The kernel reads it at load time and presents it to the user for approval.

__attribute__((section(".bz_permissions")))
static const char *permissions[] = {
    "//net:*.my-api.com/**",       // only my backend, not the whole internet
    "//user:/documents/**",        // user documents folder
    "//ram:/tmp/myapp/**",         // temporary files
    NULL
};

int main(void) {
    // By the time main() runs, the user has already approved the permissions above.
    // Any open() to a path not in this list returns -EACCES.

    int fd = open("//user:/documents/report.pdf");
    // ...
}
```

---

## Path Resolution Algorithm

When a process calls `open(path)`:

```
1. Parse scheme      → extract "//scheme:" prefix
2. Check scheme      → is this scheme read-only? reject write if so
3. Check permissions → does the process permission list match this path?
                       if not → return -EACCES
4. Resolve authority → map index/label to registered device or mount
5. Strip prefix      → pass remaining path to the filesystem driver
6. Check inode       → uid/rwx permissions on the inode
7. Return fd
```

Steps 2 and 3 happen before any filesystem driver is called. A path that fails
at step 2 or 3 never touches the disk.

---

## POSIX Compatibility Layer (future)

For porting existing Unix software, a future compatibility layer will map
standard Unix paths to Bazzulto schemes transparently:

| Unix path | Bazzulto equivalent |
|---|---|
| `/tmp/` | `//ram:/tmp/` |
| `/home/<user>/` | `//home:<user>/` |
| `~/` | `//user:/` |
| `/dev/null` | `//ram:/dev/null` |
| `/dev/sda` | `//dev:disk:0/` |
| `/proc/<pid>/` | `//proc:<pid>/` |

This layer is optional and not part of the native Bazzulto API.
