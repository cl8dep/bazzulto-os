.section .rodata
.global _user_cat_elf_start
.global _user_cat_elf_end

_user_cat_elf_start:
    .incbin "userspace/cat/cat.elf"
_user_cat_elf_end:
