.section .rodata
.global _user_head_elf_start
.global _user_head_elf_end

_user_head_elf_start:
    .incbin "userspace/head/head.elf"
_user_head_elf_end:
