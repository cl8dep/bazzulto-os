#include "../../../../include/bazzulto/console.h"
#include "../../../../include/bazzulto/physical_memory.h"
#include <stdio.h>

// ---------------------------------------------------------------------------
// Boot splash screen
//
// Displayed after all subsystems initialize and before scheduler_start().
// Clears the framebuffer and shows the OS banner with real system values.
// ---------------------------------------------------------------------------

#define BAZZULTO_VERSION "0.1.0"

// ASCII art generated with: /opt/homebrew/bin/figlet -f standard "Bazzulto"
static const char *ascii_art[] = {
    " ____                      _ _        ",
    "| __ )  __ _ _________   _| | |_ ___  ",
    "|  _ \\ / _` |_  /_  / | | | | __/ _ \\ ",
    "| |_) | (_| |/ / / /| |_| | | || (_) |",
    "|____/ \\__,_/___/___|\\__,_|_|\\__\\___/ ",
};

#define ASCII_ART_LINES 5

// Print a left-padded info row: "  label<pad>value\n"
// The label column is 14 characters wide.
static void print_field(const char *label, const char *value_str)
{
    char line[80];
    // Build "  <label>" then pad to column 16 (2 spaces + 14 chars).
    int written = ksnprintf(line, sizeof(line), "  %-14s%s", label, value_str);
    (void)written;
    console_println(line);
}

void splash_display(uint16_t shell_pid, uint32_t keyboard_irq_intid)
{
    console_clear();

    // Two blank lines of top padding.
    console_print("\n\n");

    // ASCII art banner.
    for (int line = 0; line < ASCII_ART_LINES; line++) {
        console_print(" ");
        console_println(ascii_art[line]);
    }

    console_print("\n");
    console_println("  ARM64  v" BAZZULTO_VERSION);
    console_println("  ----------------------------------------");

    // Memory.
    uint64_t total_mb  = physical_memory_total_bytes()  / (1024ULL * 1024ULL);
    uint64_t usable_mb = physical_memory_usable_bytes() / (1024ULL * 1024ULL);
    char memory_value[48];
    ksnprintf(memory_value, sizeof(memory_value),
              "%lu MB physical, %lu MB usable", total_mb, usable_mb);
    print_field("Memory", memory_value);

    // UART.
    print_field("UART", "PL011 @ 0x09000000");

    // Shell PID.
    char shell_value[32];
    ksnprintf(shell_value, sizeof(shell_value), "/bin/shell  PID %u", shell_pid);
    print_field("Shell", shell_value);

    // Keyboard — only show if a device was found (irq_intid != 0).
    if (keyboard_irq_intid != 0) {
        char keyboard_value[32];
        ksnprintf(keyboard_value, sizeof(keyboard_value),
                  "virtio-input  IRQ %u", keyboard_irq_intid);
        print_field("Keyboard", keyboard_value);
    }

    console_println("  ----------------------------------------");
    console_print("\n");
}
