// The boot entry point (_start) is compiled from start.S via build.rs.
// It is linked into .text._start, which the linker script forces to the
// very beginning of .text via KEEP(*(.text._start)).
