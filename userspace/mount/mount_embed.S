.section .rodata
.global _user_mount_elf_start
.global _user_mount_elf_end

_user_mount_elf_start:
    .incbin "userspace/mount/mount.elf"
_user_mount_elf_end:
