#pragma once

#include <stddef.h>
#include <stdint.h>
#include "scheduler.h"

// Load an ELF-64 executable and create a user-mode process.
//
// The loader:
//   1. Validates the ELF header (magic, class, endianness, architecture).
//   2. Iterates PT_LOAD program headers, allocating physical pages and
//      mapping them into a per-process TTBR0 page table with the correct
//      permissions (R/W/X from p_flags).
//   3. Allocates and maps a user stack.
//   4. Copies argv strings and pointer array onto the user stack.
//   5. Creates the process via the scheduler and adds it to the run queue.
//
// Parameters:
//   data  — pointer to the ELF file contents in kernel memory.
//   size  — size of the ELF file in bytes.
//   argv  — NULL-terminated array of argument strings (kernel pointers).
//           Pass NULL for no arguments.
//   argc  — number of arguments in argv (excluding the NULL terminator).
//
// The user stack is set up so that on entry to _start:
//   SP+0  → argc  (uint64_t)
//   SP+8  → argv[0] pointer
//   SP+16 → argv[1] pointer
//   ...
//   SP+8+8*argc → NULL
//   (string data is stored above the pointer array)
//
// Returns:
//   A pointer to the newly created process, or NULL on any validation
//   or allocation failure.
process_t *elf_loader_load(const void *data, size_t size,
                            const char *const *argv, int argc);

// Build a user address space from an ELF image without creating a process.
// Performs the same steps as elf_loader_load (validate, map segments, allocate
// stack, push argv) but returns the components needed for exec() instead of
// spawning a new scheduler entry.
//
// On success, fills:
//   *page_table_out  — the new TTBR0 page table (ready to load into TTBR0_EL1)
//   *entry_out       — the ELF entry point virtual address
//   *stack_top_out   — user SP after argv setup (16-byte aligned)
//
// Returns 0 on success, -1 on any validation or allocation failure.
// On failure the caller must NOT free partial state — the function leaks
// it (same policy as elf_loader_load on partial failure).
int elf_loader_build_image(const void *data, size_t size,
                            const char *const *argv, int argc,
                            uint64_t **page_table_out,
                            uint64_t  *entry_out,
                            uint64_t  *stack_top_out);
