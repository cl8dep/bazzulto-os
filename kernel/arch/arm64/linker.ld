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

    /* --- Executable region: mapped with PAGE_FLAGS_KERNEL_CODE --- */
    _text_start = .;
    .text : {
        *(.text .text.*)
    }

    /* Page-align between code and data so they can have different
       page table permissions (executable vs non-executable). */
    . = ALIGN(4096);
    _text_end = .;

    /* --- Non-executable region: mapped with PAGE_FLAGS_KERNEL_DATA --- */

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

    /* ELF user programs are embedded in .rodata via .incbin (hello_embed.S etc.)
       and are loaded at runtime by the ELF loader. */

    . = ALIGN(4096);
    _kernel_end = .;
}
