/* Bazzulto OS — Rust kernel linker script
   Based on kernel/arch/arm64/linker.ld.
   Added: KEEP(*(.text._start)) to ensure _start is the very first function
   regardless of object file link order. */

OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

PHDRS {
    text PT_LOAD FLAGS(5);  /* R+X */
    data PT_LOAD FLAGS(6);  /* R+W */
}

SECTIONS {
    . = 0xFFFFFFFF80000000;

    /* --- Executable region --- */
    _text_start = .;
    .text : {
        /* _start must be first — it is the CPU's first instruction after Limine. */
        KEEP(*(.text._start))
        *(.text .text.*)
    } :text

    . = ALIGN(4096);
    _text_end = .;

    /* --- Non-executable region --- */
    .rodata : {
        *(.rodata .rodata.*)
    } :data

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

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    . = ALIGN(4096);
    _kernel_end = .;
}
