#include "../../include/bazzulto/virtual_memory.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/kernel.h"
#include <stddef.h>

// A page table has 512 entries of 8 bytes each = 4096 bytes = exactly one page.
#define TABLE_ENTRY_COUNT 512

// Extract the index into each level of the page table from a virtual address.
// Each level uses 9 bits of the address (2^9 = 512 entries per table).
#define LEVEL0_INDEX(addr) (((addr) >> 39) & 0x1FF)
#define LEVEL1_INDEX(addr) (((addr) >> 30) & 0x1FF)
#define LEVEL2_INDEX(addr) (((addr) >> 21) & 0x1FF)
#define LEVEL3_INDEX(addr) (((addr) >> 12) & 0x1FF)

// Extract the physical address stored in a page table entry.
// Bits 12-47 hold the address; bits 0-11 and 48-63 are flags.
#define ENTRY_TO_PHYSICAL(entry) ((entry) & 0x0000FFFFFFFFF000ULL)

static void memory_zero(void *ptr, size_t bytes) {
    uint8_t *p = (uint8_t *)ptr;
    for (size_t i = 0; i < bytes; i++) p[i] = 0;
}

uint64_t *virtual_memory_create_table(void) {
    // Allocate one physical page for the table and zero it.
    // An entry of all zeros means "not present" — safe default.
    void *physical_page = physical_memory_alloc();
    if (!physical_page) return NULL;

    uint64_t *table = PHYSICAL_TO_VIRTUAL(physical_page);
    memory_zero(table, PAGE_SIZE);
    return table;
}

// Get or create the next-level table pointed to by a given entry.
static uint64_t *get_or_create_next_table(uint64_t *entry_ptr) {
    if (*entry_ptr & PAGE_VALID) {
        // Entry already exists — extract the physical address and convert to virtual.
        return PHYSICAL_TO_VIRTUAL(ENTRY_TO_PHYSICAL(*entry_ptr));
    }

    // Entry is empty — allocate a new table and write its address into the entry.
    void *new_table_physical = physical_memory_alloc();
    if (!new_table_physical) return NULL;

    memory_zero(PHYSICAL_TO_VIRTUAL(new_table_physical), PAGE_SIZE);

    // An intermediate entry must have both VALID and TABLE bits set.
    // The physical address is stored in bits 12-47.
    *entry_ptr = (uint64_t)new_table_physical | PAGE_VALID | PAGE_TABLE;

    return PHYSICAL_TO_VIRTUAL(new_table_physical);
}

void virtual_memory_map(uint64_t *level0_table, uint64_t virtual_addr,
                         uint64_t physical_addr, uint64_t flags) {
    // Walk the 4-level tree, creating intermediate tables as needed.
    uint64_t *level1_table = get_or_create_next_table(&level0_table[LEVEL0_INDEX(virtual_addr)]);
    if (!level1_table) return;

    uint64_t *level2_table = get_or_create_next_table(&level1_table[LEVEL1_INDEX(virtual_addr)]);
    if (!level2_table) return;

    uint64_t *level3_table = get_or_create_next_table(&level2_table[LEVEL2_INDEX(virtual_addr)]);
    if (!level3_table) return;

    // Level 3 entry points directly to the physical page, not another table.
    level3_table[LEVEL3_INDEX(virtual_addr)] = physical_addr | flags;

    // Invalidate the TLB entry for this virtual address so the CPU picks up
    // the new mapping immediately. Without this, the CPU may use a stale
    // (absent) TLB entry and fault on the next access to this address.
    // vaae1is = invalidate by virtual address, all ASIDs, EL1, inner shareable.
    __asm__ volatile(
        "dsb ishst\n"           // ensure the page table write is visible to the MMU
        "tlbi vaae1is, %0\n"   // invalidate TLB entry for this virtual page
        "dsb ish\n"             // wait for invalidation to complete
        "isb\n"                 // flush instruction pipeline
        :
        : "r"(virtual_addr >> 12)
        : "memory"
    );
}

void virtual_memory_activate(uint64_t *kernel_table) {
    // MAIR_EL1: Memory Attribute Indirection Register — ARM ARM D13.2.97
    // Defines memory type attributes referenced by the AttrIdx field in page entries.
    // Attr0 (byte 0) = 0xFF: Normal memory, Inner/Outer Write-Back Non-Transient,
    //                         Read-Allocate + Write-Allocate
    // Attr1 (byte 1) = 0x00: Device-nGnRnE (no Gathering, no Reordering, no Early ack)
    uint64_t mair = (0xFFULL << 0) | (0x00ULL << 8);

    // Read the physical address size supported by this CPU.
    // ARM ARM D13.2.52: ID_AA64MMFR0_EL1.PARange [3:0] encodes the PA width.
    // We copy this value into TCR_EL1.IPS so the MMU knows the PA output size.
    uint64_t mmfr0;
    __asm__ volatile("mrs %0, id_aa64mmfr0_el1" : "=r"(mmfr0));
    uint64_t pa_range = mmfr0 & 0xF;

    // TCR_EL1: Translation Control Register — ARM ARM D13.2.120
    //
    // WARNING: this register is written in full. Any fields left as zero take
    // effect — we must explicitly configure or disable both TTBR0 and TTBR1.
    uint64_t tcr =
        // TTBR0 region (user space) — disabled, no page table walks.
        // EPD0=1 causes any TTBR0-range access to fault immediately
        // without a table walk. We set T0SZ=16 to define the 48-bit
        // boundary even though walks are disabled.
        (16ULL << 0)   |  // T0SZ  [5:0]   = 16 → 48-bit VA split
        (1ULL  << 7)   |  // EPD0  [7]     = 1  → disable TTBR0 walks

        // TTBR1 region (kernel space) — active.
        (16ULL << 16)  |  // T1SZ  [21:16] = 16 → 48-bit VA (2^48 kernel range)
        (1ULL  << 24)  |  // IRGN1 [25:24] = 01 → inner write-back, write-allocate
        (1ULL  << 26)  |  // ORGN1 [27:26] = 01 → outer write-back, write-allocate
        (3ULL  << 28)  |  // SH1   [29:28] = 11 → inner shareable
        (2ULL  << 30)  |  // TG1   [31:30] = 10 → 4KB granule

        // IPS: Intermediate Physical Address Size — must match hardware.
        (pa_range << 32); // IPS [34:32] = from ID_AA64MMFR0_EL1.PARange

    uint64_t physical_table = VIRTUAL_TO_PHYSICAL(kernel_table);

    __asm__ volatile (
        "msr mair_el1, %0\n"
        "msr tcr_el1,  %1\n"
        "isb\n"
        "msr ttbr1_el1, %2\n"
        "isb\n"
        "tlbi vmalle1\n"
        "dsb sy\n"
        "isb\n"
        :
        : "r"(mair), "r"(tcr), "r"(physical_table)
        : "memory"
    );
}

void virtual_memory_enable_user(void) {
    // Rewrite TCR_EL1 with EPD0=0 to enable TTBR0 page table walks.
    // Also configure TTBR0 cacheability/shareability (must match MAIR Attr0).
    // Called once from kernel_main before any user process is created.
    uint64_t mmfr0;
    __asm__ volatile("mrs %0, id_aa64mmfr0_el1" : "=r"(mmfr0));
    uint64_t pa_range = mmfr0 & 0xF;

    uint64_t tcr =
        // TTBR0 region — NOW ENABLED for user space
        (16ULL << 0)   |  // T0SZ  [5:0]   = 16 → 48-bit user VA
        // EPD0 = 0 (absent) → TTBR0 walks ENABLED
        (0ULL  << 14)  |  // TG0   [15:14] = 00 → 4KB granule
        (1ULL  << 8)   |  // IRGN0 [9:8]   = 01 → inner write-back, write-allocate
        (1ULL  << 10)  |  // ORGN0 [11:10] = 01 → outer write-back, write-allocate
        (3ULL  << 12)  |  // SH0   [13:12] = 11 → inner shareable

        // TTBR1 region — unchanged
        (16ULL << 16)  |  // T1SZ
        (1ULL  << 24)  |  // IRGN1
        (1ULL  << 26)  |  // ORGN1
        (3ULL  << 28)  |  // SH1
        (2ULL  << 30)  |  // TG1 = 4KB

        (pa_range << 32); // IPS

    __asm__ volatile(
        "msr tcr_el1, %0\n"
        "isb\n"
        "tlbi vmalle1\n"
        "dsb ish\n"
        "isb\n"
        :
        : "r"(tcr)
        : "memory"
    );
}

void virtual_memory_switch_ttbr0(uint64_t *user_table) {
    uint64_t physical = VIRTUAL_TO_PHYSICAL(user_table);
    __asm__ volatile(
        "msr ttbr0_el1, %0\n"
        "isb\n"
        "tlbi vmalle1\n"     // flush all TLB entries (ASID optimization later)
        "dsb ish\n"
        "isb\n"
        :
        : "r"(physical)
        : "memory"
    );
}
