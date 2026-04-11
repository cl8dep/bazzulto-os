# CLAUDE.md — Bazzulto OS (AArch64 / QEMU virt)

## Project overview

This repository contains a hobby operating system for AArch64, but the goal is
to build it with professional standards: correct boot assumptions, documented
hardware contracts, and scalable subsystem boundaries.

The current boot path is:

1. QEMU `virt`
2. UEFI firmware (`edk2-aarch64-code.fd`)
3. Limine
4. `bazzulto.elf`
5. `_start` in `kernel/arch/arm64/boot/start.S`
6. `kernel_main()` in `kernel/arch/arm64/boot/main.c`

This is not a raw `-kernel` boot flow. Any guidance that assumes direct QEMU
kernel entry is outdated unless the repo is changed to match.

## Current repository shape

### Boot / arch

- `kernel/arch/arm64/boot/start.S`
- `kernel/arch/arm64/boot/main.c`
- `kernel/arch/arm64/linker.ld`
- `kernel/arch/arm64/exceptions/`
- `kernel/arch/arm64/timer.c`

### Memory

- `kernel/memory/physical_memory.c`
- `kernel/memory/virtual_memory.c`
- `kernel/memory/heap.c`
- `kernel/memory/MEMORY.md`

### Scheduling

- `kernel/scheduler/scheduler.c`
- `kernel/scheduler/context_switch.S`

### Drivers

- `kernel/drivers/console/console.c`

### Public headers

- `include/bazzulto/*.h`

## Toolchain and build

The current Makefile uses:

- Compiler: `aarch64-elf-gcc`
- Linker: `aarch64-elf-ld`
- Freestanding C11
- `-mgeneral-regs-only`
- No libc, no POSIX, no hosted assumptions

Supported build commands in the repo today:

```bash
make
make clean
make run
```

Do not mention `make debug`, `make dump`, or other targets unless they are
actually added to `Makefile`.

## Verified machine facts for QEMU `virt`

These values were verified from the QEMU-generated DTB for this repo's machine
configuration (`qemu-system-aarch64 -machine virt,dumpdtb=...`):

- RAM base: `0x40000000`
- GIC Distributor: `0x08000000`
- GIC CPU interface: `0x08010000`
- GICv2M frame: `0x08020000`
- PL011 UART: `0x09000000`
- PL031 RTC: `0x09010000`
- fw_cfg: `0x09020000`
- GPIO: `0x09030000`
- virtio-mmio window starts at `0x0A000000`

The dumped DTB also reports:

- `compatible = "arm,cortex-a15-gic"` for the interrupt controller
- `compatible = "arm,armv8-timer", "arm,armv7-timer"` for the architected timer
- `compatible = "arm,pl011", "arm,primecell"` for UART

If any code hardcodes QEMU `virt` MMIO addresses, it should either cite these
facts or point to the QEMU source / DTB that defines them.

## Current implemented subsystems

### Boot

- `_start` sets up a local stack and calls `kernel_main()`
- Limine requests currently used:
  - framebuffer
  - HHDM
  - memmap
  - kernel address
  - bootloader info

### Console

- Current console output uses the Limine framebuffer
- There is no early serial console yet
- The repo is therefore weaker for bring-up debugging than a PL011-first design

### Physical memory

- A simple free-list page allocator exists
- It consumes `LIMINE_MEMMAP_USABLE` pages from the Limine memmap
- It stores free-list nodes through the HHDM

### Virtual memory

- The kernel currently builds a TTBR1-only page table
- It maps:
  - the kernel image
  - HHDM-backed physical regions
  - GIC MMIO regions
- A heap region is later grown by mapping additional pages

### Exceptions

- VBAR_EL1 is installed
- A 16-entry AArch64 vector table exists
- The asm entry path saves GP registers plus ELR/SPSR/ESR/FAR
- The C side decodes EC (Exception Class) via `ec_name()` and prints the full frame

### Scheduler

- Round-robin scheduler with EL0 user processes
- Context switch saves callee-saved registers and SP
- `fork`: `scheduler_fork_process` deep-copies TTBR0 via `virtual_memory_deep_copy_table`;
  child resumes via `fork_child_resume` asm trampoline
- `exec`: `scheduler_free_user_address_space` frees old TTBR0; new image loaded
  via `elf_loader_build_image`; exception frame patched in-place

### VFS / File System

- ramfs (read-only; 128 files, 255-byte names)
- Per-process FD table (64 slots, fd 0-2 = stdin/stdout/stderr)
- Kernel pipes: 4096-byte ring buffer, yield-spin blocking, ref-count lifetime
- `dup` / `dup2` with pipe ref-count management
- `close_all_fds` on process exit

### Syscall Surface (17 syscalls)

0=exit, 1=write, 2=read, 3=yield, 4=open, 5=close, 6=seek,
7=spawn, 8=list, 9=wait, 10=pipe, 11=dup, 12=dup2,
13=mmap, 14=munmap, 15=fork, 16=exec

### User Memory

- Stack ASLR: `aslr_stack_offset()` using `CNTPCT_EL0` + page table pointer, 0–255 pages
- Stack guard page: unmapped page below the stack bottom
- Anonymous mmap: bump-pointer from `MMAP_USER_BASE = 0x200000000`, up to 16 regions/process

### Userspace libc

- `string.h`: standard + `memchr`, `strstr`, `strspn`, `strcspn`, `strpbrk`, `strtok_r`, `strtok`
- `stdlib.h`: `strtol`/`strtoll`/`strtoul`/`strtoull`, `atoi`/`atol`/`atoll`, `abs`/`labs`/`llabs`
- `stdio.h`: `printf`, `sprintf`, `fprintf`, `puts`, `putchar`

### Timer / interrupts

- The current timer path uses the architected physical timer
- The current interrupt path targets the GIC CPU interface + distributor model
- This area exists, but still requires stricter spec verification

## Current audit status

This section is important: not everything currently implemented should be
treated as architecturally correct just because it compiles.

### Verified as internally consistent

- Limine request layout and linker section placement
- Exception frame layout between asm and C
- Scheduler/context-switch struct offsets
- QEMU `virt` MMIO base addresses listed above

### Not yet fully verified against primary specs

- Full page descriptor bit layout for leaf entries (bits [11:0], [63:52] flags)
- GICv2 programming details for PPIs vs SPIs
- Timer interrupt routing assumptions (PPI INTID 30)
- Exact boot-state assumptions inherited from Limine at entry

Until the remaining items are checked against primary sources, treat them as provisional.


## Documentation standards for this repo

When changing low-level code, prefer the following source hierarchy:

1. ARM ARM / Architecture Reference Manual (`DDI 0487`)
2. GIC Architecture Specification (`IHI 0048`)
3. PL011 TRM (`DDI 0183`)
4. QEMU source for `hw/arm/virt.c`
5. Limine protocol header / Limine documentation

Do not guess register meanings, descriptor layouts, or reset-state assumptions.

### Required behavior when editing low-level code

- Every system register write should be documented
- Every magic number should either be derived from a named macro or cited
- If a mapping depends on `TCR_EL1`, `MAIR_EL1`, or descriptor format, document
  the dependency chain, not just the final constant
- If an exception is being debugged, decode `ESR_EL1` before proposing a fix
- If a value cannot be verified from a real source, say so explicitly

### Preferred local documentation layout

- `/docs/` for imported references and notes
- `/docs/wiki/` for internal project notes
- `/docs/wiki/exceptions-log.md` for exception investigations and root causes

## Current coding expectations

- Use simple, explicit C
- No libc calls
- No hidden runtime assumptions
- Prefer named constants over raw literals
- Keep assembly readable and conservative
- Match header declarations and asm struct layouts exactly
- When adding files, update `Makefile`
- Always use full, verbose names for files, functions, macros, and variables. Never abbreviate.

## Practical guidance for future work

If working on MMU, exceptions, timer, or boot:

- Start by checking the existing code against the spec, not by patching symptoms
- Trace dependencies downward:
  - linker placement
  - boot entry state
  - page-table format
  - MAIR/TCR contract
  - mapped MMIO reachability
  - exception decoding
- Treat each exception as a symptom first, not the bug itself

If working on debugging:

- Prefer adding deterministic logs over adding more moving parts
- Keep serial output in mind as a future bring-up improvement
- Correlate `ELR_EL1` with `objdump` and symbol addresses

## References already present in the repo

- `docs/ARM Instruction Set.pdf`

Note:
that PDF is not sufficient as the sole architectural reference for the current
AArch64 EL1/MMU/GIC work. Use it only as supplemental material unless the needed
topic is explicitly covered.
