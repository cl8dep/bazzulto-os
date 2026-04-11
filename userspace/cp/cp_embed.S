.section .rodata
.global _user_cp_elf_start
.global _user_cp_elf_end

_user_cp_elf_start:
    .incbin "userspace/cp/cp.elf"
_user_cp_elf_end:
