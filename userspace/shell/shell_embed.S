.section .rodata
.global _user_shell_elf_start
.global _user_shell_elf_end

_user_shell_elf_start:
    .incbin "userspace/shell/shell.elf"
_user_shell_elf_end:
