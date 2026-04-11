// Process startup code for Bazzulto OS user programs.
//
// This is the true entry point (_start) that the ELF loader jumps to.
// The kernel sets up the user stack with argc and argv before entry:
//
//   SP+0  → argc  (uint64_t)
//   SP+8  → argv[0]
//   SP+16 → argv[1]
//   ...
//   SP+8+8*argc → NULL
//
// _start loads argc and argv, calls main(argc, argv), then passes
// main's return value to sys_exit().

.text
.global _start
_start:
    // Clear frame pointer and link register for clean stack traces.
    mov x29, #0
    mov x30, #0

    // Load argc and argv from the stack (set up by the ELF loader).
    ldr x0, [sp]        // x0 = argc
    add x1, sp, #8      // x1 = &argv[0]

    // Call the program's main function.
    bl main

    // main() returned — x0 holds the exit status.
    svc #0              // SYS_EXIT(x0)

    // Unreachable.
    b .
