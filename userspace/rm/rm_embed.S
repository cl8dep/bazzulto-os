.section .rodata
.global _user_rm_elf_start
.global _user_rm_elf_end

_user_rm_elf_start:
    .incbin "userspace/rm/rm.elf"
_user_rm_elf_end:
