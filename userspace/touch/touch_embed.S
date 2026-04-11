.section .rodata
.global _user_touch_elf_start
.global _user_touch_elf_end

_user_touch_elf_start:
    .incbin "userspace/touch/touch.elf"
_user_touch_elf_end:
