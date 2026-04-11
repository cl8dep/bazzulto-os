.section .rodata
.global _user_kill_elf_start
.global _user_kill_elf_end

_user_kill_elf_start:
    .incbin "userspace/kill/kill.elf"
_user_kill_elf_end:
