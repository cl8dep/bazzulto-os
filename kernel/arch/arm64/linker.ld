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
        EXCLUDE_FILE(user/test_program.o user/echo.o) *(.text .text.*)
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

    /* Embedded user-mode programs (copied into per-process page tables at runtime).
       Stored in the kernel image, mapped at USER_TEXT_BASE (0x00400000) at runtime. */
    . = ALIGN(4096);
    _user_text_start = .;
    .user_text : {
        . = ALIGN(4);
        _user_hello_start = .;
        user/test_program.o(.text)
        _user_hello_end = .;

        . = ALIGN(4);
        _user_echo_start = .;
        user/echo.o(.text)
        _user_echo_end = .;
    }
    _user_text_end = .;

    . = ALIGN(4096);
    _kernel_end = .;
}
