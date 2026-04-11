.section .rodata
.global _user_tee_elf_start
.global _user_tee_elf_end

_user_tee_elf_start:
    .incbin "userspace/tee/tee.elf"
_user_tee_elf_end:
