.section .rodata
.global _user_hexdump_elf_start
.global _user_hexdump_elf_end

_user_hexdump_elf_start:
    .incbin "userspace/hexdump/hexdump.elf"
_user_hexdump_elf_end:
