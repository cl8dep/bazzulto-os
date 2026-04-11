.section .rodata
.global _user_df_elf_start
.global _user_df_elf_end

_user_df_elf_start:
    .incbin "userspace/df/df.elf"
_user_df_elf_end:
