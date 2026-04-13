# Contributing to Bazzulto OS

Thank you for your interest in contributing. This document describes the
standards and process for contributing code to this project.

## Before you start

- Read [README.md](README.md) to understand the architecture.
- Check open issues before opening a new one — your bug or idea may already
  be tracked.
- For large changes, open an issue first to discuss the approach before
  writing code. This avoids wasted effort if the direction doesn't fit.

## Development setup

```bash
# Clone the repo
git clone <repo-url>
cd bazzulto

# Full build
make

# Run in QEMU
make run
```

See the README for the full list of prerequisites.

## Code standards

### General

- **Verbose names** — no abbreviations in functions, variables, macros, or
  files. `physical_address` not `phys_addr`, `page_table_entry` not `pte`.
- **No legacy hardware** — UEFI only. No BIOS, no PS/2.
- **No magic numbers** — every hardware constant must be derived from a named
  macro with a source citation (ARM ARM, GIC spec, QEMU DTB, etc.).
- **No guessed layouts** — every register field and descriptor format must
  cite the relevant spec section.

### Rust (kernel and userspace)

- `#![no_std]` everywhere — no hosted assumptions.
- `unsafe` blocks must have a `// Safety:` comment explaining the invariant.
- Use `core::ptr::addr_of_mut!` instead of `&mut STATIC` for mutable statics.
- Panic handler must be defined in every binary (`#[panic_handler]`).
- No `unwrap()` on paths reachable from user input or external data.

### Assembly (AArch64)

- Every system register write must have a comment citing the ARM ARM section.
- Callee-saved registers (x19–x28) must be saved and restored by any
  function that uses them.
- Exception frame layout in `.S` files must match `ExceptionFrame` in Rust
  exactly — the compile-time size assertion enforces this.

### Documentation

Follow the source hierarchy when citing hardware behavior:

1. ARM Architecture Reference Manual (DDI 0487)
2. GIC Architecture Specification (IHI 0048)
3. PL011 TRM (DDI 0183)
4. QEMU source `hw/arm/virt.c`
5. Limine protocol specification

## Pull request process

1. Branch off `main`: `git checkout -b your-feature`
2. Keep commits small and focused — one logical change per commit.
3. Write a clear commit message: what changed and *why*.
4. Run `make` and `make run` before opening the PR to confirm it boots.
5. Open the PR against `main` with a description of the change and how to
   test it.
6. Address review feedback before merging.

## Reporting bugs

Open a GitHub issue with:

- What you expected to happen
- What actually happened (include UART output / exception frame if applicable)
- Exact QEMU command used
- Commit SHA

## Code of Conduct

All contributors are expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
