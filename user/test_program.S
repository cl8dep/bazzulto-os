// Bazzulto OS - EL0 test program
//
// This is the first user-space program. It writes a message to stdout
// via the sys_write syscall, then exits via sys_exit.
//
// Assembled with -Ttext=0x400000 so PC-relative addressing (adr)
// produces correct offsets when mapped at USER_TEXT_BASE.
//
// Syscall convention:
//   x0-x5 = arguments
//   SVC #nr = syscall number
//   x0 = return value

.text
.global _user_start

_user_start:
    // sys_write(fd=1, buf=msg, len=17)
    mov  x0, #1              // fd = stdout
    adr  x1, msg             // buf = PC-relative address of message
    mov  x2, #17             // len = strlen("Hello from EL0!\n")
    svc  #1                  // SYS_WRITE

    // sys_exit(status=0)
    mov  x0, #0              // status = 0
    svc  #0                  // SYS_EXIT

    // Should never reach here
    b    .

msg:
    .ascii "Hello from EL0!\n\0"
