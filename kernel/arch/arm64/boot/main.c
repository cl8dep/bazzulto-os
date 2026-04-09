#include "../../../../include/bazzulto/console.h"
#include "../../../../include/bazzulto/heap.h"
#include "../../../../include/bazzulto/kernel.h"
#include "../../../../include/bazzulto/physical_memory.h"
#include "../../../../include/bazzulto/virtual_memory.h"
#include "../../../../limine/limine.h"

__attribute__((used, section(".limine_requests")))
static volatile struct limine_framebuffer_request framebuffer_request = {
    .id = LIMINE_FRAMEBUFFER_REQUEST,
    .revision = 0
};

__attribute__((used, section(".limine_requests")))
static volatile struct limine_hhdm_request hhdm_request = {
    .id = LIMINE_HHDM_REQUEST,
    .revision = 0
};

__attribute__((used, section(".limine_requests")))
static volatile struct limine_memmap_request memmap_request = {
    .id = LIMINE_MEMMAP_REQUEST,
    .revision = 0
};

// Ask Limine where it loaded the kernel in both physical and virtual memory.
// We need this to map the kernel into our own page table before activating it.
__attribute__((used, section(".limine_requests")))
static volatile struct limine_kernel_address_request kernel_address_request = {
    .id = LIMINE_KERNEL_ADDRESS_REQUEST,
    .revision = 0
};

__attribute__((used, section(".limine_requests")))
static volatile struct limine_bootloader_info_request bootloader_info_request = {
    .id = LIMINE_BOOTLOADER_INFO_REQUEST,
    .revision = 0
};

__attribute__((used, section(".limine_requests_start")))
static volatile LIMINE_REQUESTS_START_MARKER;

__attribute__((used, section(".limine_requests_end")))
static volatile LIMINE_REQUESTS_END_MARKER;

uint64_t hhdm_offset = 0;

// The kernel's page table — exposed so other subsystems can map new pages.
uint64_t *kernel_page_table = NULL;

// Print a size_t as decimal — temporary until we have console_printf.
static void print_number(size_t n) {
    char digits[20];
    int length = 0;
    if (n == 0) { digits[length++] = '0'; }
    else { while (n > 0) { digits[length++] = '0' + (n % 10); n /= 10; } }
    for (int a = 0, b = length - 1; a < b; a++, b--) {
        char tmp = digits[a]; digits[a] = digits[b]; digits[b] = tmp;
    }
    digits[length] = '\0';
    console_print(digits);
}

void kernel_main(void) {
    if (!framebuffer_request.response || framebuffer_request.response->framebuffer_count < 1) {
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

    // --- Step 1: store HHDM offset ---
    if (!hhdm_request.response) {
        console_println("FATAL: no HHDM response");
        for (;;) __asm__("wfe");
    }
    hhdm_offset = hhdm_request.response->offset;
    console_println("HHDM: ok");

    // --- Step 2: initialize physical memory allocator ---
    if (!memmap_request.response) {
        console_println("FATAL: no memory map");
        for (;;) __asm__("wfe");
    }
    physical_memory_init(memmap_request.response);
    console_print("Physical memory: ");
    print_number(physical_memory_free_page_count());
    console_println(" pages free");

    // --- Step 3: build our own kernel page table ---
    if (!kernel_address_request.response) {
        console_println("FATAL: no kernel address response");
        for (;;) __asm__("wfe");
    }

    uint64_t kernel_physical_base = kernel_address_request.response->physical_base;
    uint64_t kernel_virtual_base  = kernel_address_request.response->virtual_base;

    uint64_t *kernel_table = virtual_memory_create_table();
    if (!kernel_table) {
        console_println("FATAL: could not allocate page table");
        for (;;) __asm__("wfe");
    }

    // Map the entire kernel image (we use 4MB as a safe upper bound for now).
    // Each page is mapped with the same virtual→physical offset Limine set up.
    uint64_t kernel_size = 4 * 1024 * 1024;
    for (uint64_t offset = 0; offset < kernel_size; offset += PAGE_SIZE) {
        virtual_memory_map(kernel_table,
                           kernel_virtual_base  + offset,
                           kernel_physical_base + offset,
                           PAGE_FLAGS_KERNEL_CODE);
    }

    // Map every usable physical region via the HHDM so PHYSICAL_TO_VIRTUAL
    // keeps working after we switch tables. We iterate the memory map instead
    // of hardcoding a size — this works regardless of how much RAM is present.
    struct limine_memmap_response *memmap = memmap_request.response;
    for (uint64_t i = 0; i < memmap->entry_count; i++) {
        struct limine_memmap_entry *entry = memmap->entries[i];
        uint64_t base = entry->base & ~(uint64_t)(PAGE_SIZE - 1); // align down
        uint64_t end  = entry->base + entry->length;
        for (uint64_t addr = base; addr < end; addr += PAGE_SIZE) {
            virtual_memory_map(kernel_table,
                               hhdm_offset + addr,
                               addr,
                               PAGE_FLAGS_KERNEL_DATA);
        }
    }

    // --- Step 4: activate our page table ---
    kernel_page_table = kernel_table;
    virtual_memory_activate(kernel_table);
    console_println("Virtual memory: active");

    // --- Step 5: initialize the kernel heap ---
    heap_init();
    console_println("Heap: ok");

    console_println("Kernel initialized. Halting.");
    for (;;) __asm__("wfe");
}
