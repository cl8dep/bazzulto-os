// Bazzulto OS - EL0 echo program
//
// Reads one character at a time from stdin (sys_read) and writes it
// back to stdout (sys_write). Demonstrates interactive I/O from user space.
//
// Syscall convention: x0-x2 = args, svc #nr, x0 = return value.

.text
.global _echo_start

_echo_start:
    // Print prompt
    mov  x0, #1              // fd = stdout
    adr  x1, prompt          // buf
    mov  x2, #16             // len
    svc  #1                  // sys_write

_loop:
    // sys_read(0, buf, 1) — read one char from stdin (blocks via UART IRQ)
    // Use the stack as a writable buffer (user code pages are read-only).
    sub  sp, sp, #16         // allocate 16 bytes on stack (aligned)
    mov  x1, sp              // buf = stack
    mov  x2, #1              // len = 1
    svc  #2                  // sys_read

    // sys_write(1, buf, 1) — echo the character back
    mov  x0, #1              // fd = stdout
    mov  x1, sp              // buf = same stack slot
    mov  x2, #1              // len = 1
    svc  #1                  // sys_write

    // Check if it was Enter (\r) — if so, also write \n
    ldrb w3, [sp]
    add  sp, sp, #16         // free stack space
    cmp  w3, #13             // '\r'
    b.ne _loop

    // Write newline
    sub  sp, sp, #16
    mov  w4, #10             // '\n'
    strb w4, [sp]
    mov  x0, #1
    mov  x1, sp
    mov  x2, #1
    svc  #1                  // sys_write
    add  sp, sp, #16
    b    _loop

prompt:
    .ascii "echo@bazzulto> \0"
