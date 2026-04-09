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
    // MAIR_EL1: Memory Attribute Indirection Register.
    // Defines memory type attributes referenced by the AttrIdx field in page entries.
    // Index 0 = Normal memory, write-back cacheable (0xFF)
    // Index 1 = Device memory, nGnRnE — strictly ordered, no caching (0x00)
    uint64_t mair = (0xFFULL << 0) | (0x00ULL << 8);

    // TCR_EL1: Translation Control Register.
    // T1SZ=16 → TTBR1 covers the top 2^(64-16) = 2^48 of address space (0xFFFF...)
    // TG1=10  → 4KB granule for TTBR1
    // SH1=11  → inner shareable
    // ORGN1=01, IRGN1=01 → write-back, read-allocate cacheable
    uint64_t tcr = (16ULL << 16) |  // T1SZ
                   (2ULL  << 30) |  // TG1: 4KB granule
                   (3ULL  << 28) |  // SH1: inner shareable
                   (1ULL  << 26) |  // ORGN1: write-back cacheable
                   (1ULL  << 24);   // IRGN1: write-back cacheable

    uint64_t physical_table = VIRTUAL_TO_PHYSICAL(kernel_table);

    __asm__ volatile (
        "msr mair_el1, %0\n"    // Set memory attribute types
        "msr tcr_el1,  %1\n"    // Set address space size and caching policy
        "isb\n"                  // Ensure both registers are visible before loading table
        "msr ttbr1_el1, %2\n"   // Load kernel page table (physical address)
        "isb\n"
        "tlbi vmalle1\n"         // Invalidate all TLB entries for EL1
        "dsb sy\n"               // Wait for TLB invalidation to complete
        "isb\n"
        :
        : "r"(mair), "r"(tcr), "r"(physical_table)
        : "memory"
    );
}
