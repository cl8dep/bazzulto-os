# Bazzulto OS

Bazzulto is an AArch64 operating system built from scratch in Rust, designed
with professional standards: correct boot assumptions, documented hardware
contracts, and well-defined subsystem boundaries.

## Architecture

```
QEMU virt  →  UEFI (edk2)  →  Limine  →  bazzulto.elf (Rust kernel)
                                              ↓
                                         bzinit (PID 1, service manager)
                                              ↓
                                     bzsh / bzdisplayd / drivers …
```

The kernel is pure Rust (`kernel/`). All userspace — including the init system,
shell, display server, and utilities — is also Rust (`userspace/bin/`), built
against the Bazzulto Standard Library (`userspace/lib/bsl/`).

## Features

- **UEFI boot** via Limine — no BIOS, no legacy hardware
- **Pure Rust kernel** — AArch64 EL1, no C
- **ELF64 loader** with ASLR, guard pages, and 8 MiB user stacks
- **vDSO** — syscall numbers are never baked into compiled binaries
- **VFS** — ramfs, FAT32, tmpfs, devfs, procfs
- **Slab allocator** — 10 power-of-two size classes, 64 KB arenas
- **Round-robin scheduler** with 32 768 PID slots
- **bzinit** — dependency-ordered service manager (PID 1)
- **bzsh** — interactive shell
- **bzdisplayd** — userspace display server via framebuffer

## Building

**Prerequisites:**

- `aarch64-elf-gcc` / `aarch64-elf-ld` (cross toolchain)
- Rust nightly with `aarch64-unknown-none` target
- QEMU (`qemu-system-aarch64`) with `edk2-aarch64-code.fd`
- Python 3 (for `create_disk.py`)

```bash
# Full build (BSL release + kernel debug)
make

# Run in QEMU
make run

# Release build + run
make run-release

# Build a single userspace crate
make bsl-crate CRATE=bzinit

# Clean everything
make clean
```

## Project layout

```
kernel/          Rust kernel source
userspace/
  bin/           Rust userspace workspace (bzinit, bzsh, bzctl, …)
  lib/bsl/       Bazzulto Standard Library
    Bazzulto.System/   syscall wrappers, global allocator, vDSO
    Bazzulto.IO/       file, path, stream abstractions
  libraries/
    libc/        POSIX C library headers (stubs) + musl submodule
  services/      TOML service definition files
  fonts/         Embedded fonts
esp/             UEFI boot partition (EFI + Limine config)
docs/            References and internal wiki
```

## Third-party components

### musl libc

Bazzulto includes [musl](https://musl.libc.org/) as a git submodule at
`userspace/libraries/libc/musl/`.

musl is an MIT-licensed implementation of the standard C library written by
Rich Felker and contributors. It targets the Linux syscall API; the Bazzulto
port replaces `arch/aarch64/bits/syscall.h.in` with Bazzulto syscall numbers
while keeping the rest of musl unchanged.

- Homepage: <https://musl.libc.org/>
- Repository: <https://git.musl-libc.org/cgit/musl>
- License: MIT — see `userspace/libraries/libc/musl/COPYRIGHT`

## License

MIT — see [LICENSE](LICENSE).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
