.section .rodata
.global _user_wc_elf_start
.global _user_wc_elf_end

_user_wc_elf_start:
    .incbin "userspace/wc/wc.elf"
_user_wc_elf_end:
