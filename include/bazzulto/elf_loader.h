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
