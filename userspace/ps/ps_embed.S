.section .rodata
.global _user_ps_elf_start
.global _user_ps_elf_end

_user_ps_elf_start:
    .incbin "userspace/ps/ps.elf"
_user_ps_elf_end:
