#include "../../../../include/bazzulto/console.h"
#include <stdio.h>
#include "../../../../include/bazzulto/exceptions.h"
#include "../../../../include/bazzulto/heap.h"
#include "../../../../include/bazzulto/scheduler.h"
#include "../../../../include/bazzulto/timer.h"
#include "../../../../include/bazzulto/uart.h"
#include "../../../../include/bazzulto/kernel.h"
#include "../../../../include/bazzulto/physical_memory.h"
#include "../../../../include/bazzulto/virtual_memory.h"
#include "../../../../include/bazzulto/ramfs.h"
#include "../../../../include/bazzulto/elf_loader.h"
#include "../../../../include/bazzulto/systemcall.h"
#include "../../../../include/bazzulto/input.h"
#include "../../../../include/bazzulto/virtio_mmio.h"
#include "../../../../include/bazzulto/keyboard.h"
#include "../../../../include/bazzulto/splash.h"
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
    char physical_memory_message[64];
    ksnprintf(physical_memory_message, sizeof(physical_memory_message),
              "Physical memory: %lu pages free",
              (unsigned long)physical_memory_free_page_count());
    console_println(physical_memory_message);

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

    // Map the kernel image using linker-exported section boundaries.
    // .text is mapped executable; everything else is non-executable (W^X).
    // Physical addresses are derived from the VA→PA offset Limine provides.
    extern char _text_start[], _text_end[], _kernel_end[];
    uint64_t phys_offset = kernel_physical_base - kernel_virtual_base;

    for (uint64_t va = (uint64_t)_text_start; va < (uint64_t)_text_end; va += PAGE_SIZE) {
        virtual_memory_map(kernel_table, va, va + phys_offset, PAGE_FLAGS_KERNEL_CODE);
    }
    for (uint64_t va = (uint64_t)_text_end; va < (uint64_t)_kernel_end; va += PAGE_SIZE) {
        virtual_memory_map(kernel_table, va, va + phys_offset, PAGE_FLAGS_KERNEL_DATA);
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

    // Map GIC MMIO regions as device memory so timer_init can access them after
    // we activate our page table. These are MMIO addresses — not RAM — so they
    // won't appear in the Limine memory map. We must add them explicitly.
    // GIC Distributor: 0x08000000, size 64KB (16 pages)
    // GIC CPU Interface: 0x08010000, size 64KB (16 pages)
    for (uint64_t page = 0; page < 16; page++) {
        uint64_t gicd_page = 0x08000000ULL + page * PAGE_SIZE;
        uint64_t gicc_page = 0x08010000ULL + page * PAGE_SIZE;
        virtual_memory_map(kernel_table, hhdm_offset + gicd_page, gicd_page, PAGE_FLAGS_KERNEL_DEVICE);
        virtual_memory_map(kernel_table, hhdm_offset + gicc_page, gicc_page, PAGE_FLAGS_KERNEL_DEVICE);
    }

    // Map PL011 UART MMIO — physical 0x09000000, one page (4KB).
    // QEMU virt machine: first UART at this address (hw/arm/virt.c).
    virtual_memory_map(kernel_table,
                       hhdm_offset + 0x09000000ULL,
                       0x09000000ULL,
                       PAGE_FLAGS_KERNEL_DEVICE);

    // Map virtio-mmio slots as device memory — physical 0x0A000000, 32 slots
    // of 0x200 bytes each = 0x4000 bytes total = 4 pages.
    // QEMU virt: virtio-mmio region at 0x0A000000, size 0x4000 (hw/arm/virt.c).
    for (uint64_t page = 0; page < 4; page++) {
        uint64_t virtio_page = VIRTIO_MMIO_BASE + page * PAGE_SIZE;
        virtual_memory_map(kernel_table,
                           hhdm_offset + virtio_page,
                           virtio_page,
                           PAGE_FLAGS_KERNEL_DEVICE);
    }

    // --- Step 4: activate our page table ---
    kernel_page_table = kernel_table;
    virtual_memory_activate(kernel_table);
    console_println("Virtual memory: active");

    // --- Step 5: initialize the kernel heap ---
    heap_init();
    console_println("Heap: ok");

    // --- Step 6: initialize ramfs ---
    ramfs_init();

    // --- Step 7: install exception vector table ---
    exceptions_init();

    // --- Step 8: initialize scheduler, timer, and UART ---
    // Timer must init first — it sets up the GIC distributor and CPU interface.
    // UART init depends on a working GIC to register IRQ 33.
    scheduler_init();
    timer_init();

    uart_init();
    uart_puts("UART: ok\n");
    console_println("UART: ok");

    // Initialize the input abstraction layer before any driver that feeds it.
    input_init();

    // Enumerate virtio-mmio devices so keyboard_init() can find the input device.
    virtio_mmio_enumerate();

    // Initialize the virtio-input keyboard driver. Safe to call when no
    // keyboard device is present (QEMU run-serial target) — logs a warning
    // and continues with serial-only input via the UART IRQ path.
    keyboard_init();

    // Enable TTBR0 page table walks for user-space processes.
    virtual_memory_enable_user();

    // Register all ELF programs in ramfs.
    extern char _user_hello_elf_start[], _user_hello_elf_end[];
    extern char _user_shell_elf_start[], _user_shell_elf_end[];
    extern char _user_ls_elf_start[],    _user_ls_elf_end[];
    extern char _user_help_elf_start[],  _user_help_elf_end[];
    extern char _user_echo_elf_start[],  _user_echo_elf_end[];

    ramfs_register("/bin/hello", _user_hello_elf_start,
                   _user_hello_elf_end - _user_hello_elf_start);
    ramfs_register("/bin/shell", _user_shell_elf_start,
                   _user_shell_elf_end - _user_shell_elf_start);
    ramfs_register("/bin/ls", _user_ls_elf_start,
                   _user_ls_elf_end - _user_ls_elf_start);
    ramfs_register("/bin/help", _user_help_elf_start,
                   _user_help_elf_end - _user_help_elf_start);
    ramfs_register("/bin/echo", _user_echo_elf_start,
                   _user_echo_elf_end - _user_echo_elf_start);

    // Launch the shell as the initial user-mode process (the init process).
    // Register its PID as the orphan reaper before starting the scheduler so
    // that any process that dies before the shell spawns a child is reparented
    // correctly rather than leaking as a permanent zombie.
    uint16_t shell_pid = 0;
    {
        process_t *shell = elf_loader_load(_user_shell_elf_start,
                                           _user_shell_elf_end - _user_shell_elf_start,
                                           NULL, 0);
        if (shell) {
            systemcall_set_init_process(shell->pid.index);
            shell_pid = shell->pid.index;
        } else {
            console_println("FATAL: failed to load shell");
        }
    }

    splash_display(shell_pid, keyboard_get_irq_intid());
    scheduler_start();  // does not return

    console_println("Kernel initialized. Halting.");
    for (;;) __asm__("wfe");
}
