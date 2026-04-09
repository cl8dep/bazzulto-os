#include "../../../../include/bazzulto/console.h"
#include "../../../../limine/limine.h"

// Tell Limine we want framebuffer access.
// Limine scans the kernel binary for these request structs by their magic IDs
// and fills in the .response pointer before jumping to _start.
__attribute__((used, section(".limine_requests")))
static volatile struct limine_framebuffer_request framebuffer_request = {
    .id = LIMINE_FRAMEBUFFER_REQUEST,
    .revision = 0
};

// Ask Limine for its own name and version string, so we can log it at boot.
__attribute__((used, section(".limine_requests")))
static volatile struct limine_bootloader_info_request bootloader_info_request = {
    .id = LIMINE_BOOTLOADER_INFO_REQUEST,
    .revision = 0
};

// Required by the Limine protocol: marks the start and end of the requests section.
__attribute__((used, section(".limine_requests_start")))
static volatile LIMINE_REQUESTS_START_MARKER;

__attribute__((used, section(".limine_requests_end")))
static volatile LIMINE_REQUESTS_END_MARKER;

void kernel_main(void) {
    // Verify Limine gave us a framebuffer before using it
    if (!framebuffer_request.response || framebuffer_request.response->framebuffer_count < 1) {
        // No framebuffer: nothing we can do, halt
        for (;;) __asm__("wfe");
    }

    struct limine_framebuffer *fb = framebuffer_request.response->framebuffers[0];
    console_init(fb);

    console_println("Bazzulto OS booting...");

    if (bootloader_info_request.response) {
        console_print("Bootloader: ");
        console_print(bootloader_info_request.response->name);
        console_print(" ");
        console_println(bootloader_info_request.response->version);
    }

    console_println("Kernel initialized. Halting.");

    for (;;) __asm__("wfe");
}
