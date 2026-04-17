/* BSL userspace linker script.
 * Forces 4 KiB segment alignment so the kernel ELF loader (which walks
 * PT_LOAD segments page-by-page using the system page size of 4 KiB) maps
 * each segment correctly without an intra-page offset bug.
 *
 * Without this, the default aarch64 lld max-page-size of 64 KiB causes
 * .text to start at a VMA that is NOT 4 KiB aligned, causing the loader's
 * saturating_sub to zero out the intra-page offset and map file data to the
 * wrong virtual addresses.
 */

PHDRS {
    rodata PT_LOAD FLAGS(4);   /* R */
    text   PT_LOAD FLAGS(5);   /* R+X */
    data   PT_LOAD FLAGS(6);   /* R+W */
}

SECTIONS {
    . = 0x200000;

    .rodata : ALIGN(0x1000) {
        *(.rodata .rodata.*)
        *(.eh_frame_hdr)
        *(.eh_frame)
    } :rodata

    .text : ALIGN(0x1000) {
        *(.text .text.*)
        *(.init)
        *(.fini)
    } :text

    .data : ALIGN(0x1000) {
        *(.got .got.*)
        *(.data .data.*)
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.gnu.version*)
        *(.note*)
    }
}
