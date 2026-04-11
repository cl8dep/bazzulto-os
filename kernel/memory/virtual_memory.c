#include "../../include/bazzulto/virtual_memory.h"
#include <string.h>
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


uint64_t *virtual_memory_create_table(void) {
    // Allocate one physical page for the table and zero it.
    // An entry of all zeros means "not present" — safe default.
    void *physical_page = physical_memory_alloc();
    if (!physical_page) return NULL;

    uint64_t *table = PHYSICAL_TO_VIRTUAL(physical_page);
    memset(table, 0, PAGE_SIZE);
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

    memset(PHYSICAL_TO_VIRTUAL(new_table_physical), 0, PAGE_SIZE);

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
    // -----------------------------------------------------------------------
    // MAIR_EL1 — Memory Attribute Indirection Register
    // ARM ARM DDI 0487 D17.2.97 (AArch64 System register descriptions)
    //
    // The register is an 8-byte array: MAIR_EL1[7:0] = Attr0, [15:8] = Attr1, …
    // Each page descriptor's AttrIdx[2:0] (bits [4:2]) indexes into this array.
    // We define two attributes, matching PAGE_ATTR_NORMAL (AttrIdx=0) and
    // PAGE_ATTR_DEVICE (AttrIdx=1) in virtual_memory.h.
    //
    // Attr0 = 0xFF  →  Normal memory, Write-Back Non-Transient, RA+WA (both inner and outer)
    //   encoding: bits [7:4] = outer attributes = 0b1111 (WB, non-transient, RA+WA)
    //             bits [3:0] = inner attributes = 0b1111 (WB, non-transient, RA+WA)
    //   Reference: DDI 0487 D17.2.97, Table D17-28 (Attr encoding for Normal memory).
    //   This is the fastest, most cache-friendly type — suitable for kernel code/data/heap.
    //
    // Attr1 = 0x00  →  Device-nGnRnE
    //   encoding: 0b0000_0000 (bits [7:4] = 0b0000 → Device; bits [3:0] = 0b0000 → nGnRnE)
    //   Reference: DDI 0487 D17.2.97, Table D17-28 (Attr encoding for Device memory).
    //   nGnRnE = no Gathering, no Reordering, no Early Write Acknowledgement.
    //   Strongest ordering for MMIO — prevents the CPU from combining or reordering
    //   accesses, which is required for registers with side effects (GIC, UART, timer).
    // -----------------------------------------------------------------------
    uint64_t mair = (0xFFULL << 0)   // Attr0: Normal WB RA+WA
                  | (0x00ULL << 8);  // Attr1: Device nGnRnE

    // -----------------------------------------------------------------------
    // ID_AA64MMFR0_EL1 — read hardware-reported physical address size
    // ARM ARM DDI 0487 D17.2.64
    // PARange [3:0] encodes the maximum PA size supported:
    //   0b0000 = 32 bits, 0b0001 = 36 bits, 0b0010 = 40 bits,
    //   0b0011 = 42 bits, 0b0100 = 44 bits, 0b0101 = 48 bits, 0b0110 = 52 bits.
    // We copy this directly into TCR_EL1.IPS (same encoding).
    // QEMU virt with cortex-a72 reports 0b0101 (48-bit PA).
    // -----------------------------------------------------------------------
    uint64_t mmfr0;
    __asm__ volatile("mrs %0, id_aa64mmfr0_el1" : "=r"(mmfr0));
    uint64_t pa_range = mmfr0 & 0xF;

    // -----------------------------------------------------------------------
    // TCR_EL1 — Translation Control Register
    // ARM ARM DDI 0487 D17.2.131
    //
    // CRITICAL: this register is written in full. Fields left as zero take effect.
    // We must configure both TTBR0 (user) and TTBR1 (kernel) halves explicitly.
    //
    // VA split:
    //   T0SZ = T1SZ = 16  →  both halves cover 2^(64-16) = 2^48 bytes.
    //   TTBR0 handles VAs [0x0000_0000_0000_0000 … 0x0000_FFFF_FFFF_FFFF].
    //   TTBR1 handles VAs [0xFFFF_0000_0000_0000 … 0xFFFF_FFFF_FFFF_FFFF].
    //   Reference: DDI 0487 D17.2.131, T0SZ/T1SZ fields.
    //
    // Granule size:
    //   TG0 = 0b00, TG1 = 0b10  →  both 4KB granules.
    //   Reference: DDI 0487 D17.2.131, TG0[15:14], TG1[31:30].
    //   4KB chosen because it matches the physical allocator page size and is
    //   the most common granule on QEMU virt / Cortex-A72.
    //   Note: TG1 encoding is INVERTED from TG0: 0b10 = 4KB, 0b01 = 16KB, 0b11 = 64KB.
    //
    // Cacheability (TTBR1 — kernel walks):
    //   IRGN1 [25:24] = 0b01  →  Inner Write-Back, Read-Allocate, Write-Allocate.
    //   ORGN1 [27:26] = 0b01  →  Outer Write-Back, Read-Allocate, Write-Allocate.
    //   Reference: DDI 0487 D17.2.131, IRGN1/ORGN1 fields.
    //   These control cacheability of the page table walk itself (the intermediate
    //   table reads), not the final page it points to. WB-RA-WA is the standard
    //   cacheable choice and matches Attr0 in MAIR.
    //
    // Shareability (TTBR1):
    //   SH1 [29:28] = 0b11  →  Inner Shareable.
    //   Reference: DDI 0487 D17.2.131, SH1 field; D8.3.5 shareability model.
    //   Inner Shareable means all observers in the same Inner Shareable domain
    //   (all CPUs on a typical SoC) see coherent data. Required for SMP correctness
    //   and safe even on single-core (no downside).
    //
    // TTBR0 (user space) — DISABLED at kernel init time:
    //   EPD0 [7] = 1  →  Translation table walk for TTBR0 is DISABLED.
    //   Reference: DDI 0487 D17.2.131, EPD0 field.
    //   Any access to a TTBR0-range address will fault immediately without a walk.
    //   This is the safe state while no user process is running. User walks are
    //   re-enabled by virtual_memory_enable_user() before the first eret to EL0.
    //   T0SZ=16 is set so the boundary is well-defined even while disabled.
    //
    // IPS [34:32] — Intermediate Physical Address size:
    //   Copied from ID_AA64MMFR0_EL1.PARange (same encoding).
    //   Reference: DDI 0487 D17.2.131, IPS field.
    //   Must match the hardware PA capability. Mismatch causes UNDEFINED behavior
    //   when the MMU tries to output a PA wider than the configured IPS.
    // -----------------------------------------------------------------------
    uint64_t tcr =
        (16ULL << 0)   |  // T0SZ  [5:0]   = 16 → 48-bit TTBR0 VA range
        (1ULL  << 7)   |  // EPD0  [7]     = 1  → TTBR0 walks disabled until EL0 is ready

        (16ULL << 16)  |  // T1SZ  [21:16] = 16 → 48-bit TTBR1 VA range (kernel)
        (1ULL  << 24)  |  // IRGN1 [25:24] = 01 → kernel table walk: inner WB RA+WA
        (1ULL  << 26)  |  // ORGN1 [27:26] = 01 → kernel table walk: outer WB RA+WA
        (3ULL  << 28)  |  // SH1   [29:28] = 11 → kernel table walk: inner shareable
        (2ULL  << 30)  |  // TG1   [31:30] = 10 → 4KB granule for TTBR1

        (pa_range << 32); // IPS   [34:32]       → from ID_AA64MMFR0_EL1.PARange

    uint64_t physical_table = VIRTUAL_TO_PHYSICAL(kernel_table);

    // Write MAIR first, then TCR, then TTBR1.
    // The ISB after TCR ensures the new TCR values are visible before TTBR1
    // is written and before any TLB invalidation.
    // "dsb sy" after tlbi ensures invalidation completes system-wide.
    // The final ISB flushes the pipeline so the next instruction sees the new state.
    // Reference: DDI 0487 D8.11 (TLB maintenance instructions and ordering).
    __asm__ volatile (
        "msr mair_el1, %0\n"
        "msr tcr_el1,  %1\n"
        "isb\n"
        "msr ttbr1_el1, %2\n"
        "isb\n"
        "tlbi vmalle1\n"    // invalidate all TLB entries (all ASIDs, EL1)
        "dsb sy\n"          // system-wide barrier — wait for TLB invalidation
        "isb\n"
        :
        : "r"(mair), "r"(tcr), "r"(physical_table)
        : "memory"
    );
}

void virtual_memory_enable_user(void) {
    // Rewrite TCR_EL1 to clear EPD0, enabling TTBR0 page table walks.
    // Also configure the TTBR0 half's cacheability and shareability
    // (should match the TTBR1 settings and MAIR Attr0 for consistency).
    // Called once from kernel_main() immediately before the first eret to EL0.
    //
    // Changes from virtual_memory_activate():
    //   - EPD0 [7] = 0 (absent from the constant) → TTBR0 walks ENABLED.
    //     Reference: DDI 0487 D17.2.131, EPD0 field.
    //   - TG0  [15:14] = 0b00 → 4KB granule for TTBR0.
    //     Reference: DDI 0487 D17.2.131, TG0 field; 0b00 = 4KB (unlike TG1 where 0b10 = 4KB).
    //   - IRGN0 [9:8]  = 0b01 → inner WB RA+WA (same policy as IRGN1).
    //   - ORGN0 [11:10]= 0b01 → outer WB RA+WA.
    //   - SH0   [13:12]= 0b11 → inner shareable (same policy as SH1).
    //
    // TTBR1 fields are preserved at the same values as in virtual_memory_activate().
    uint64_t mmfr0;
    __asm__ volatile("mrs %0, id_aa64mmfr0_el1" : "=r"(mmfr0));
    uint64_t pa_range = mmfr0 & 0xF;

    uint64_t tcr =
        // TTBR0 (user space) — EPD0 absent → walks enabled
        (16ULL << 0)   |  // T0SZ  [5:0]   = 16 → 48-bit user VA
        (0ULL  << 14)  |  // TG0   [15:14] = 00 → 4KB granule (0b00 = 4KB for TG0)
        (1ULL  << 8)   |  // IRGN0 [9:8]   = 01 → user table walk: inner WB RA+WA
        (1ULL  << 10)  |  // ORGN0 [11:10] = 01 → user table walk: outer WB RA+WA
        (3ULL  << 12)  |  // SH0   [13:12] = 11 → user table walk: inner shareable

        // TTBR1 (kernel space) — identical to virtual_memory_activate()
        (16ULL << 16)  |  // T1SZ  [21:16] = 16
        (1ULL  << 24)  |  // IRGN1 [25:24] = 01
        (1ULL  << 26)  |  // ORGN1 [27:26] = 01
        (3ULL  << 28)  |  // SH1   [29:28] = 11
        (2ULL  << 30)  |  // TG1   [31:30] = 10 → 4KB (0b10 = 4KB for TG1)

        (pa_range << 32); // IPS   [34:32]

    // ISB after msr ensures the new TCR takes effect before the TLB flush.
    // dsb ish waits for inner-shareable domain to see the invalidation complete
    // (weaker than "dsb sy" but sufficient for a single-cluster machine).
    // Reference: DDI 0487 D8.11.
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

int kernel_vm_alloc(uint64_t vaddr, uint64_t n_pages, uint64_t flags) {
    for (uint64_t i = 0; i < n_pages; i++) {
        void *physical_page = physical_memory_alloc();
        if (!physical_page) return -1;
        virtual_memory_map(kernel_page_table,
                           vaddr + i * PAGE_SIZE,
                           (uint64_t)physical_page,
                           flags);
    }
    return 0;
}

void virtual_memory_unmap_range(uint64_t *page_table,
                                uint64_t vaddr, uint64_t n_pages)
{
    for (uint64_t i = 0; i < n_pages; i++) {
        uint64_t va = vaddr + i * PAGE_SIZE;

        uint64_t l0_entry = page_table[LEVEL0_INDEX(va)];
        if (!(l0_entry & PAGE_VALID)) continue;

        uint64_t *l1 = (uint64_t *)PHYSICAL_TO_VIRTUAL(ENTRY_TO_PHYSICAL(l0_entry));
        uint64_t l1_entry = l1[LEVEL1_INDEX(va)];
        if (!(l1_entry & PAGE_VALID)) continue;

        uint64_t *l2 = (uint64_t *)PHYSICAL_TO_VIRTUAL(ENTRY_TO_PHYSICAL(l1_entry));
        uint64_t l2_entry = l2[LEVEL2_INDEX(va)];
        if (!(l2_entry & PAGE_VALID)) continue;

        uint64_t *l3 = (uint64_t *)PHYSICAL_TO_VIRTUAL(ENTRY_TO_PHYSICAL(l2_entry));
        uint64_t l3_entry = l3[LEVEL3_INDEX(va)];
        if (!(l3_entry & PAGE_VALID)) continue;

        physical_memory_free((void *)ENTRY_TO_PHYSICAL(l3_entry));
        l3[LEVEL3_INDEX(va)] = 0;

        __asm__ volatile(
            "dsb ishst\n"
            "tlbi vaae1is, %0\n"
            "dsb ish\n"
            "isb\n"
            : : "r"(va >> 12) : "memory");
    }
}

uint64_t *virtual_memory_deep_copy_table(const uint64_t *src_l0)
{
    uint64_t *dst_l0 = virtual_memory_create_table();
    if (!dst_l0) return NULL;

    // Iterate all 512 L0 entries.
    // User space only occupies the lower half (TTBR0): indices 0-255 for a
    // 48-bit VA split. Index 256+ belong to the kernel TTBR1 range — skip them.
    for (int i0 = 0; i0 < 256; i0++) {
        uint64_t l0_entry = src_l0[i0];
        if (!(l0_entry & PAGE_VALID)) continue;

        const uint64_t *src_l1 = (const uint64_t *)PHYSICAL_TO_VIRTUAL(
                                      ENTRY_TO_PHYSICAL(l0_entry));

        uint64_t *dst_l1 = virtual_memory_create_table();
        if (!dst_l1) return NULL;
        dst_l0[i0] = (uint64_t)VIRTUAL_TO_PHYSICAL(dst_l1) | PAGE_VALID | PAGE_TABLE;

        for (int i1 = 0; i1 < TABLE_ENTRY_COUNT; i1++) {
            uint64_t l1_entry = src_l1[i1];
            if (!(l1_entry & PAGE_VALID)) continue;

            const uint64_t *src_l2 = (const uint64_t *)PHYSICAL_TO_VIRTUAL(
                                          ENTRY_TO_PHYSICAL(l1_entry));

            uint64_t *dst_l2 = virtual_memory_create_table();
            if (!dst_l2) return NULL;
            dst_l1[i1] = (uint64_t)VIRTUAL_TO_PHYSICAL(dst_l2) | PAGE_VALID | PAGE_TABLE;

            for (int i2 = 0; i2 < TABLE_ENTRY_COUNT; i2++) {
                uint64_t l2_entry = src_l2[i2];
                if (!(l2_entry & PAGE_VALID)) continue;

                const uint64_t *src_l3 = (const uint64_t *)PHYSICAL_TO_VIRTUAL(
                                              ENTRY_TO_PHYSICAL(l2_entry));

                uint64_t *dst_l3 = virtual_memory_create_table();
                if (!dst_l3) return NULL;
                dst_l2[i2] = (uint64_t)VIRTUAL_TO_PHYSICAL(dst_l3) | PAGE_VALID | PAGE_TABLE;

                for (int i3 = 0; i3 < TABLE_ENTRY_COUNT; i3++) {
                    uint64_t l3_entry = src_l3[i3];
                    if (!(l3_entry & PAGE_VALID)) continue;

                    // Allocate a fresh physical page and copy the contents.
                    void *new_phys = physical_memory_alloc();
                    if (!new_phys) return NULL;

                    void *src_page = PHYSICAL_TO_VIRTUAL(ENTRY_TO_PHYSICAL(l3_entry));
                    void *dst_page = PHYSICAL_TO_VIRTUAL(new_phys);
                    memcpy(dst_page, src_page, PAGE_SIZE);

                    // Preserve the original page flags (permissions, cacheability, etc.)
                    // but replace the physical address with the new page.
                    uint64_t flags = l3_entry & ~0x0000FFFFFFFFF000ULL;
                    dst_l3[i3] = (uint64_t)new_phys | flags;
                }
            }
        }
    }

    return dst_l0;
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
