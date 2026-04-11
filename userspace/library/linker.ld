/* Linker script for Bazzulto OS user-space programs.
   Programs are loaded at 0x00400000 by the ELF loader.

   Two ELF LOAD segments with distinct permissions eliminate the RWX warning.
   PT_LOAD FLAGS: PF_R=4, PF_W=2, PF_X=1
     text  = R+X (5) — executable code
     data  = R+W (6) — rodata, data, bss */
OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

PHDRS {
    text PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0x00400000;

    .text : {
        *(.text .text.*)
    } :text

    /* Page-align between code and data for distinct segment permissions. */
    . = ALIGN(4096);

    .rodata : {
        *(.rodata .rodata.*)
    } :data

    .data : {
        *(.data .data.*)
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data
}
