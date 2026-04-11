.section .rodata
.global _user_sleep_elf_start
.global _user_sleep_elf_end

_user_sleep_elf_start:
    .incbin "userspace/sleep/sleep.elf"
_user_sleep_elf_end:
