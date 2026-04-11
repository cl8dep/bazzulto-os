.section .rodata
.global _user_grep_elf_start
.global _user_grep_elf_end

_user_grep_elf_start:
    .incbin "userspace/grep/grep.elf"
_user_grep_elf_end:
