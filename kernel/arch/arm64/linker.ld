/* Bazzulto OS - ARM64 linker script
   Defines the memory layout of the kernel binary.
   Limine loads the kernel at a high virtual address (kernel space starts at 0xFFFFFFFF80000000). */

OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

/* Two ELF LOAD segments with distinct permissions eliminate the RWX warning.
   PT_LOAD FLAGS: PF_R=4, PF_W=2, PF_X=1
     text  = R+X (5) — only executable code
     data  = R+W (6) — rodata, data, bss (non-executable) */
PHDRS {
    text PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    /* Place the kernel at -2GB in virtual address space, a common convention
       that keeps kernel addresses clearly separate from userspace. */
    . = 0xFFFFFFFF80000000;

    /* --- Executable region: mapped with PAGE_FLAGS_KERNEL_CODE --- */
    _text_start = .;
    .text : {
        *(.text .text.*)
    } :text

    /* Page-align between code and data so they can have different
       page table permissions (executable vs non-executable). */
    . = ALIGN(4096);
    _text_end = .;

    /* --- Non-executable region: mapped with PAGE_FLAGS_KERNEL_DATA --- */

    .rodata : {
        *(.rodata .rodata.*)
    } :data

    /* Limine scans this section to find boot requests declared in the kernel. */
    .limine_requests_start : {
        KEEP(*(.limine_requests_start))
    } :data
    .limine_requests : {
        KEEP(*(.limine_requests))
    } :data
    .limine_requests_end : {
        KEEP(*(.limine_requests_end))
    } :data

    .data : {
        *(.data .data.*)
    } :data

    /* BSS holds zero-initialized globals (including our kernel stack).
       It is not stored in the file — the loader zeroes it at runtime. */
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* ELF user programs are embedded in .rodata via .incbin (hello_embed.S etc.)
       and are loaded at runtime by the ELF loader. */

    . = ALIGN(4096);
    _kernel_end = .;
}
