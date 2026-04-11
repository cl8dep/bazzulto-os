.section .rodata
.global _user_help_elf_start
.global _user_help_elf_end

_user_help_elf_start:
    .incbin "userspace/help/help.elf"
_user_help_elf_end:
