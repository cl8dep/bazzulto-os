#pragma once

// ELF-64 Object File Format definitions.
// Reference: Tool Interface Standard (TIS) Executable and Linking Format (ELF)
//            Specification, Version 1.2 — plus ELF-64 extensions.
//
// Only the subset needed for loading static AArch64 executables is defined here.

#include <stdint.h>

// --- ELF identification (e_ident) indices and values ---

#define ELF_IDENT_SIZE     16

#define ELF_IDENT_MAGIC0   0   // 0x7F
#define ELF_IDENT_MAGIC1   1   // 'E'
#define ELF_IDENT_MAGIC2   2   // 'L'
#define ELF_IDENT_MAGIC3   3   // 'F'
#define ELF_IDENT_CLASS    4   // File class
#define ELF_IDENT_DATA     5   // Data encoding (endianness)
#define ELF_IDENT_VERSION  6   // ELF version
#define ELF_IDENT_OSABI    7   // OS/ABI identification

// e_ident[ELF_IDENT_CLASS]
#define ELF_CLASS_64       2   // 64-bit objects

// e_ident[ELF_IDENT_DATA]
#define ELF_DATA_LITTLE_ENDIAN 1   // Little-endian (AArch64 default)

// e_ident[ELF_IDENT_VERSION]
#define ELF_VERSION_CURRENT 1

// --- ELF header fields ---

// e_type: object file type
#define ELF_TYPE_EXECUTABLE 2  // Executable file

// e_machine: target architecture
#define ELF_MACHINE_AARCH64 183  // ARM AARCH64

// --- Program header types (p_type) ---

#define ELF_PROGRAM_TYPE_NULL    0  // Unused entry
#define ELF_PROGRAM_TYPE_LOAD    1  // Loadable segment

// --- Program header flags (p_flags) ---

#define ELF_PROGRAM_FLAG_EXECUTE 0x1  // Segment is executable
#define ELF_PROGRAM_FLAG_WRITE   0x2  // Segment is writable
#define ELF_PROGRAM_FLAG_READ    0x4  // Segment is readable

// --- ELF-64 header ---
// TIS ELF Spec, Figure 1-3 (adapted for 64-bit)

typedef struct {
    uint8_t  e_ident[ELF_IDENT_SIZE]; // ELF identification
    uint16_t e_type;                   // Object file type
    uint16_t e_machine;                // Architecture
    uint32_t e_version;                // Object file version
    uint64_t e_entry;                  // Entry point virtual address
    uint64_t e_phoff;                  // Program header table file offset
    uint64_t e_shoff;                  // Section header table file offset
    uint32_t e_flags;                  // Processor-specific flags
    uint16_t e_ehsize;                 // ELF header size in bytes
    uint16_t e_phentsize;              // Program header table entry size
    uint16_t e_phnum;                  // Program header table entry count
    uint16_t e_shentsize;              // Section header table entry size
    uint16_t e_shnum;                  // Section header table entry count
    uint16_t e_shstrndx;              // Section name string table index
} elf64_header_t;

// --- ELF-64 program header ---
// TIS ELF Spec, Figure 2-1 (adapted for 64-bit)

typedef struct {
    uint32_t p_type;    // Segment type
    uint32_t p_flags;   // Segment flags (read/write/execute)
    uint64_t p_offset;  // Segment file offset
    uint64_t p_vaddr;   // Segment virtual address
    uint64_t p_paddr;   // Segment physical address (unused for user-space)
    uint64_t p_filesz;  // Segment size in file
    uint64_t p_memsz;   // Segment size in memory (>= p_filesz; excess is zeroed for .bss)
    uint64_t p_align;   // Segment alignment
} elf64_program_header_t;
