.section .rodata
.global _user_echo_elf_start
.global _user_echo_elf_end

_user_echo_elf_start:
    .incbin "userspace/echo/echo.elf"
_user_echo_elf_end:
