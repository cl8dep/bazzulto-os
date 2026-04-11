// Embeds the hello.elf user program into the kernel binary.
// The ELF file is included as raw data in the .rodata section.
// kernel_main registers this blob in ramfs so the ELF loader can find it.

.section .rodata
.global _user_hello_elf_start
.global _user_hello_elf_end

_user_hello_elf_start:
    .incbin "userspace/hello/hello.elf"
_user_hello_elf_end:
