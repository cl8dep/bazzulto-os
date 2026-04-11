/* Linker script for Bazzulto OS user-space programs.
   Programs are loaded at 0x00400000 by the ELF loader. */

OUTPUT_FORMAT("elf64-littleaarch64")
OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS {
    . = 0x00400000;

    .text : {
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
}
