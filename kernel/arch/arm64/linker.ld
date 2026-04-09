/* Bazzulto OS - ARM64 linker script
   Defines the memory layout of the kernel binary.
   Limine loads the kernel at a high virtual address (kernel space starts at 0xFFFFFFFF80000000). */

OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS {
    /* Place the kernel at -2GB in virtual address space, a common convention
       that keeps kernel addresses clearly separate from userspace. */
    . = 0xFFFFFFFF80000000;

    .text : {
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    /* Limine scans this section to find boot requests declared in the kernel. */
    .limine_requests_start : {
        KEEP(*(.limine_requests_start))
    }
    .limine_requests : {
        KEEP(*(.limine_requests))
    }
    .limine_requests_end : {
        KEEP(*(.limine_requests_end))
    }

    .data : {
        *(.data .data.*)
    }

    /* BSS holds zero-initialized globals (including our kernel stack).
       It is not stored in the file — the loader zeroes it at runtime. */
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
}
