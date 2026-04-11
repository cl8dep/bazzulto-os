.section .rodata
.global _user_ls_elf_start
.global _user_ls_elf_end

_user_ls_elf_start:
    .incbin "userspace/ls/ls.elf"
_user_ls_elf_end:
