// memory/virtual_memory.rs — AArch64 4-level page table manager.
//
// Reference: ARM ARM DDI 0487, Chapter D5 "The AArch64 Virtual Memory System Architecture".
//
// Compared to the C implementation:
// - `map()` returns `Result<(), MapError>` instead of void — no silent failures.
// - Intermediate table allocation failure rolls back (frees already-allocated
//   tables) before returning Err — no leaks.
// - Physical address mask calculated at runtime from ID_AA64MMFR0_EL1.PARange
//   instead of being hardcoded to 48-bit.
// - `deep_copy_table()` rolls back all allocations on any failure.
// - `kernel_vm_alloc()` propagates errors instead of returning -1/0.

use super::address::{PhysicalAddress, VirtualAddress};
use super::physical::PhysicalAllocator;

// ---------------------------------------------------------------------------
// Page descriptor bit definitions
// Reference: ARM ARM DDI 0487 D5.3.3, Table D5-34.
// ---------------------------------------------------------------------------

/// Bit 0: descriptor is valid/present.
const PAGE_VALID: u64 = 1 << 0;
/// Bit 1: page descriptor (L3) or table pointer (L0-L2).
const PAGE_TABLE: u64 = 1 << 1;
/// Bit 10: Access Flag — must be set or CPU raises an AF fault.
const PAGE_ACCESS_FLAG: u64 = 1 << 10;
/// Bit 53: Privileged Execute Never — prevents EL1 execution.
const PAGE_PXN: u64 = 1 << 53;
/// Bit 54: User Execute Never — prevents EL0 execution.
const PAGE_UXN: u64 = 1 << 54;

/// AttrIdx=0 → MAIR byte 0: Normal WB cacheable.  Bits [4:2] = 0b000.
const PAGE_ATTR_NORMAL: u64 = 0 << 2;
/// AttrIdx=1 → MAIR byte 1: Device nGnRnE.  Bits [4:2] = 0b001.
const PAGE_ATTR_DEVICE: u64 = 1 << 2;

/// AP[2:1]=00 (bits [7:6]): EL1 R/W, EL0 no access.
const PAGE_KERNEL_READWRITE: u64 = 0 << 6;
/// AP[2:1]=01 (bits [7:6]): EL1+EL0 R/W.
const PAGE_USER_READWRITE: u64 = 1 << 6;
/// AP[2:1]=11 (bits [7:6]): EL1+EL0 read-only.
const PAGE_USER_READONLY: u64 = 3 << 6;

/// Bit 55 (software-defined, ignored by hardware): Copy-on-Write marker.
///
/// Set by `cow_copy_user()` on L3 leaf entries that are made read-only for
/// CoW sharing between parent and child.  Cleared when the page is privatised
/// (copied on first write fault).
///
/// Reference: ARM ARM DDI 0487, D8.3 Table D8-60, bits [58:55] are
/// "IGNORED" (software-defined) for stage-1 non-contiguous leaf descriptors.
const PAGE_COW: u64 = 1 << 55;

/// SH[1:0]=11 (bits [9:8]): Inner Shareable.
const PAGE_SH_INNER: u64 = 3 << 8;
/// SH[1:0]=10 (bits [9:8]): Outer Shareable.
const PAGE_SH_OUTER: u64 = 2 << 8;

/// Block descriptor flags — bit 1 cleared (block, not table pointer).
///
/// Level-2 block descriptors map 2 MiB regions.  Bit 1 = 0 distinguishes a
/// block descriptor from a table descriptor (bit 1 = 1).
/// Reference: ARM ARM DDI 0487 D8.3, Table D8-51, "Block descriptor, VMSAv8-64".
const PAGE_BLOCK_VALID: u64 = PAGE_VALID; // bit 0 = 1, bit 1 = 0

/// Kernel .data / heap mapped as 2 MiB block (HHDM optimisation).
///
/// Same attributes as PAGE_FLAGS_KERNEL_DATA except bit 1 = 0 (block entry).
/// Used for 2 MiB-aligned regions in the HHDM to reduce page-table depth
/// (one L2 entry covers 512 L3 entries, saving 4 KiB of table memory per
/// 2 MiB mapped and reducing TLB pressure significantly).
pub const PAGE_FLAGS_KERNEL_DATA_BLOCK: u64 = PAGE_BLOCK_VALID
    | PAGE_ACCESS_FLAG
    | PAGE_KERNEL_READWRITE
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_PXN
    | PAGE_UXN;

/// Device MMIO mapped as 2 MiB block.
pub const PAGE_FLAGS_KERNEL_DEVICE_BLOCK: u64 = PAGE_BLOCK_VALID
    | PAGE_ACCESS_FLAG
    | PAGE_KERNEL_READWRITE
    | PAGE_ATTR_DEVICE
    | PAGE_SH_OUTER
    | PAGE_PXN
    | PAGE_UXN;

/// Size of a 2 MiB huge page block (used for HHDM mapping).
pub const BLOCK_SIZE_2MIB: u64 = 2 * 1024 * 1024;

/// Composite flags for the most common mappings.

/// Kernel .text: EL1 executable, EL0 cannot execute.
pub const PAGE_FLAGS_KERNEL_CODE: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_KERNEL_READWRITE
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_UXN;

/// Kernel .data / .bss / heap / stack: not executable by anyone.
pub const PAGE_FLAGS_KERNEL_DATA: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_KERNEL_READWRITE
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_PXN
    | PAGE_UXN;

/// Device MMIO (GIC, UART, framebuffer): Device-nGnRnE, Outer Shareable.
pub const PAGE_FLAGS_KERNEL_DEVICE: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_KERNEL_READWRITE
    | PAGE_ATTR_DEVICE
    | PAGE_SH_OUTER
    | PAGE_PXN
    | PAGE_UXN;

/// User .text: read-only from both EL1 and EL0; EL0 may execute; EL1 may not.
pub const PAGE_FLAGS_USER_CODE: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_USER_READONLY
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_PXN;

/// User read-only data (e.g. vDSO data page): read-only from EL0, not executable.
pub const PAGE_FLAGS_USER_DATA_READ_ONLY: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_USER_READONLY
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_PXN
    | PAGE_UXN;

/// User stack / heap: R/W for EL1 and EL0, not executable.
pub const PAGE_FLAGS_USER_DATA: u64 = PAGE_VALID
    | PAGE_TABLE
    | PAGE_ACCESS_FLAG
    | PAGE_USER_READWRITE
    | PAGE_ATTR_NORMAL
    | PAGE_SH_INNER
    | PAGE_PXN
    | PAGE_UXN;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// Physical allocator returned None — out of memory.
    OutOfPhysicalMemory,
    /// The virtual address is not correctly aligned.
    UnalignedAddress,
}

// ---------------------------------------------------------------------------
// Runtime PA mask from ID_AA64MMFR0_EL1.PARange
// ---------------------------------------------------------------------------

/// Read the physical address size supported by the CPU from ID_AA64MMFR0_EL1.
///
/// PARange [3:0] encoding:
///   0b0000 = 32 bits  (4 GiB)
///   0b0001 = 36 bits  (64 GiB)
///   0b0010 = 40 bits  (1 TiB)
///   0b0011 = 42 bits  (4 TiB)
///   0b0100 = 44 bits  (16 TiB)
///   0b0101 = 48 bits  (256 TiB)  ← QEMU virt / Cortex-A72
///   0b0110 = 52 bits  (4 PiB)
///
/// Reference: ARM ARM DDI 0487 D17.2.64, ID_AA64MMFR0_EL1.
fn read_pa_bits() -> u8 {
    let mmfr0: u64;
    unsafe { core::arch::asm!("mrs {}, id_aa64mmfr0_el1", out(reg) mmfr0) };
    match mmfr0 & 0xF {
        0 => 32,
        1 => 36,
        2 => 40,
        3 => 42,
        4 => 44,
        5 => 48,
        6 => 52,
        _ => 48, // unknown — safe fallback
    }
}

/// Mask that extracts the physical address from a page table entry.
///
/// Bits [pa_bits-1 : 12] hold the output address; bits [11:0] and [63:pa_bits] are flags.
fn pa_mask() -> u64 {
    let pa_bits = read_pa_bits();
    ((1u64 << pa_bits) - 1) & !0xFFF
}

// ---------------------------------------------------------------------------
// Index extraction
// ---------------------------------------------------------------------------

#[inline]
fn level0_index(addr: u64) -> usize { ((addr >> 39) & 0x1FF) as usize }
#[inline]
fn level1_index(addr: u64) -> usize { ((addr >> 30) & 0x1FF) as usize }
#[inline]
fn level2_index(addr: u64) -> usize { ((addr >> 21) & 0x1FF) as usize }
#[inline]
fn level3_index(addr: u64) -> usize { ((addr >> 12) & 0x1FF) as usize }

// ---------------------------------------------------------------------------
// Table helpers
// ---------------------------------------------------------------------------

const TABLE_ENTRY_COUNT: usize = 512;

/// Allocate a zeroed page table.  Returns a virtual pointer to the table and
/// the physical address of the table (needed for page table entries).
fn create_table(
    allocator: &mut PhysicalAllocator,
) -> Option<(*mut u64, PhysicalAddress)> {
    let phys = allocator.alloc()?;
    let virt = phys.to_virtual(allocator.hhdm_offset());
    let ptr = virt.as_ptr::<u64>();
    // Zero the table — a zeroed entry means "not present".
    unsafe { core::ptr::write_bytes(ptr, 0, TABLE_ENTRY_COUNT) };
    Some((ptr, phys))
}

/// Given an existing valid intermediate entry, return a virtual pointer to the
/// next-level table it points to.
fn entry_to_table_ptr(entry: u64, hhdm_offset: u64, mask: u64) -> *mut u64 {
    let phys = PhysicalAddress::new(entry & mask);
    phys.to_virtual(hhdm_offset).as_ptr::<u64>()
}

// ---------------------------------------------------------------------------
// PageTable — wrapper around the L0 table physical address
// ---------------------------------------------------------------------------

pub struct PageTable {
    /// Physical address of the L0 (root) table.  Written directly to TTBR1_EL1.
    root_phys: PhysicalAddress,
    /// HHDM offset — stored here so all methods are self-contained.
    hhdm_offset: u64,
}

impl PageTable {
    /// Allocate and zero a new, empty L0 table.
    pub fn new(allocator: &mut PhysicalAllocator) -> Result<Self, MapError> {
        let (_ptr, phys) =
            create_table(allocator).ok_or(MapError::OutOfPhysicalMemory)?;
        Ok(Self {
            root_phys: phys,
            hhdm_offset: allocator.hhdm_offset(),
        })
    }

    fn root_ptr(&self) -> *mut u64 {
        self.root_phys
            .to_virtual(self.hhdm_offset)
            .as_ptr::<u64>()
    }

    /// Map a single virtual page to a physical page.
    ///
    /// Intermediate tables are allocated as needed.  If any allocation fails,
    /// all tables created during *this call* are freed before returning `Err`.
    /// This prevents leaks on partial failures.
    ///
    /// Reference: Linux mm/__handle_mm_fault — rollback strategy.
    pub fn map(
        &mut self,
        virtual_address: VirtualAddress,
        physical_address: PhysicalAddress,
        flags: u64,
        allocator: &mut PhysicalAllocator,
    ) -> Result<(), MapError> {
        let va = virtual_address.as_u64();
        let mask = pa_mask();

        // Track tables allocated during this call for rollback.
        let mut newly_allocated: [Option<PhysicalAddress>; 3] = [None; 3];
        let mut allocated_count = 0usize;

        let l0 = self.root_ptr();

        // Helper closure — get or create a next-level table.
        // Records any new allocation for potential rollback.
        macro_rules! get_or_create {
            ($entry_ptr:expr, $idx:expr) => {{
                let entry_ptr: *mut u64 = $entry_ptr;
                let entry = unsafe { *entry_ptr };
                if entry & PAGE_VALID != 0 {
                    // Entry already present — follow it.
                    entry_to_table_ptr(entry, self.hhdm_offset, mask)
                } else {
                    // Allocate new intermediate table.
                    match create_table(allocator) {
                        Some((ptr, phys)) => {
                            newly_allocated[allocated_count] = Some(phys);
                            allocated_count += 1;
                            unsafe {
                                *entry_ptr = phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                            }
                            ptr
                        }
                        None => {
                            // Free any tables created during this map() call.
                            for i in 0..allocated_count {
                                if let Some(p) = newly_allocated[i] {
                                    unsafe { allocator.free_page(p) };
                                }
                            }
                            return Err(MapError::OutOfPhysicalMemory);
                        }
                    }
                }
            }};
        }

        let l0_entry = unsafe { l0.add(level0_index(va)) };
        let l1 = get_or_create!(l0_entry, level0_index(va));

        let l1_entry = unsafe { l1.add(level1_index(va)) };
        let l2 = get_or_create!(l1_entry, level1_index(va));

        let l2_entry = unsafe { l2.add(level2_index(va)) };
        let l3 = get_or_create!(l2_entry, level2_index(va));

        // Write the leaf entry.
        unsafe {
            *l3.add(level3_index(va)) = physical_address.as_u64() | flags;
        }

        // Invalidate the TLB entry for this virtual address.
        // dsb ishst: ensure the page table write is visible to the MMU.
        // tlbi vaae1is: invalidate by VA, all ASIDs, EL1, inner-shareable domain.
        // dsb ish: wait for the invalidation to complete.
        // isb: flush the instruction pipeline.
        // Reference: ARM ARM DDI 0487 D8.11 (TLB maintenance instructions).
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vaae1is, {va}",
                "dsb ish",
                "isb",
                va = in(reg) va >> 12,
                options(nostack, preserves_flags)
            );
        }

        Ok(())
    }

    /// Map a 2 MiB block at L2 using a block descriptor.
    ///
    /// Both `virtual_address` and `physical_address` must be 2 MiB aligned.
    /// The L2 entry is written directly with a block descriptor (bit 1 = 0),
    /// bypassing L3 entirely.  This uses `flags` from `PAGE_FLAGS_KERNEL_DATA_BLOCK`
    /// or `PAGE_FLAGS_KERNEL_DEVICE_BLOCK` (both have bit 1 = 0).
    ///
    /// If the L2 slot already contains a table pointer, this call is skipped
    /// (returns `Ok(())`) to avoid replacing a fine-grained mapping with a block.
    ///
    /// Reference: ARM ARM DDI 0487 D8.3 Table D8-51 (L2 block descriptor).
    pub fn map_block(
        &mut self,
        virtual_address: VirtualAddress,
        physical_address: PhysicalAddress,
        flags: u64,
        allocator: &mut PhysicalAllocator,
    ) -> Result<(), MapError> {
        let va = virtual_address.as_u64();
        let mask = pa_mask();

        // 2 MiB alignment check.
        if va & (BLOCK_SIZE_2MIB - 1) != 0 || physical_address.as_u64() & (BLOCK_SIZE_2MIB - 1) != 0 {
            return Err(MapError::UnalignedAddress);
        }

        let l0 = self.root_ptr();

        // Walk to L2 entry, allocating L0 and L1 tables as needed.
        macro_rules! get_or_create {
            ($entry_ptr:expr) => {{
                let entry_ptr: *mut u64 = $entry_ptr;
                let entry = unsafe { *entry_ptr };
                if entry & PAGE_VALID != 0 {
                    if entry & PAGE_TABLE == 0 {
                        // Already a block descriptor at L1 — our region is
                        // covered by a coarser mapping; skip silently.
                        return Ok(());
                    }
                    entry_to_table_ptr(entry, self.hhdm_offset, mask)
                } else {
                    match create_table(allocator) {
                        Some((ptr, phys)) => {
                            unsafe {
                                *entry_ptr = phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                            }
                            ptr
                        }
                        None => return Err(MapError::OutOfPhysicalMemory),
                    }
                }
            }};
        }

        let l0_entry_ptr = unsafe { l0.add(level0_index(va)) };
        let l1 = get_or_create!(l0_entry_ptr);

        let l1_entry_ptr = unsafe { l1.add(level1_index(va)) };
        let l2 = get_or_create!(l1_entry_ptr);

        let l2_entry_ptr = unsafe { l2.add(level2_index(va)) };
        let l2_entry = unsafe { *l2_entry_ptr };

        // If this L2 slot already has a table pointer (fine-grained mapping),
        // don't overwrite it with a block — the fine-grained mapping wins.
        if l2_entry & PAGE_VALID != 0 && l2_entry & PAGE_TABLE != 0 {
            return Ok(());
        }

        // Write the L2 block descriptor.
        // Physical address bits [47:21] placed at bits [47:21] of the descriptor.
        // Bits [20:0] of the descriptor are attribute bits (flags).
        // The `flags` parameter must already have bit 1 = 0 (block, not table).
        unsafe {
            *l2_entry_ptr = physical_address.as_u64() | flags;
        }

        // TLB invalidation.
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vaae1is, {va}",
                "dsb ish",
                "isb",
                va = in(reg) va >> 12,
                options(nostack, preserves_flags)
            );
        }

        Ok(())
    }

    /// Clear a single page's L3 entry without freeing the physical page.
    ///
    /// Returns the physical address that was mapped at `virtual_address`, or
    /// `None` if the page was not mapped.
    ///
    /// Used for kernel stack guard pages: the heap still owns the physical
    /// memory; only the mapping is removed so that accesses cause a Data Abort,
    /// catching stack overflow before it silently corrupts kernel data.
    ///
    /// The caller is responsible for restoring the mapping (via `map()`) before
    /// the heap deallocates the containing allocation, so that the heap can write
    /// its free-list metadata into the block.
    pub fn unmap_no_free(&mut self, virtual_address: VirtualAddress) -> Option<PhysicalAddress> {
        let va = virtual_address.as_u64();
        let mask = pa_mask();
        let l0 = self.root_ptr();

        let l0_entry = unsafe { *l0.add(level0_index(va)) };
        if l0_entry & PAGE_VALID == 0 { return None; }
        let l1 = entry_to_table_ptr(l0_entry, self.hhdm_offset, mask);

        let l1_entry = unsafe { *l1.add(level1_index(va)) };
        if l1_entry & PAGE_VALID == 0 { return None; }
        let l2 = entry_to_table_ptr(l1_entry, self.hhdm_offset, mask);

        let l2_entry = unsafe { *l2.add(level2_index(va)) };
        if l2_entry & PAGE_VALID == 0 { return None; }
        let l3 = entry_to_table_ptr(l2_entry, self.hhdm_offset, mask);

        let l3_entry_ptr = unsafe { l3.add(level3_index(va)) };
        let l3_entry = unsafe { *l3_entry_ptr };
        if l3_entry & PAGE_VALID == 0 { return None; }

        let phys = PhysicalAddress::new(l3_entry & mask);
        unsafe { *l3_entry_ptr = 0 };

        // TLB invalidation — same sequence as `map()`.
        // Reference: ARM ARM DDI 0487 D8.11.
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vaae1is, {va}",
                "dsb ish",
                "isb",
                va = in(reg) va >> 12,
                options(nostack, preserves_flags)
            );
        }

        Some(phys)
    }

    /// Unmap a range of `page_count` pages starting at `virtual_address`,
    /// freeing the underlying physical pages.
    ///
    /// Intermediate tables are NOT freed (same policy as the C implementation).
    pub fn unmap_range(
        &mut self,
        virtual_address: VirtualAddress,
        page_count: usize,
        page_size: u64,
        allocator: &mut PhysicalAllocator,
    ) {
        let mask = pa_mask();

        for i in 0..page_count {
            let va = virtual_address.as_u64() + i as u64 * page_size;
            let l0 = self.root_ptr();

            // Walk the table, checking each level for validity.
            let l0_entry = unsafe { *l0.add(level0_index(va)) };
            if l0_entry & PAGE_VALID == 0 { continue; }

            let l1 = entry_to_table_ptr(l0_entry, self.hhdm_offset, mask);
            let l1_entry = unsafe { *l1.add(level1_index(va)) };
            if l1_entry & PAGE_VALID == 0 { continue; }

            let l2 = entry_to_table_ptr(l1_entry, self.hhdm_offset, mask);
            let l2_entry = unsafe { *l2.add(level2_index(va)) };
            if l2_entry & PAGE_VALID == 0 { continue; }

            let l3 = entry_to_table_ptr(l2_entry, self.hhdm_offset, mask);
            let l3_entry_ptr = unsafe { l3.add(level3_index(va)) };
            let l3_entry = unsafe { *l3_entry_ptr };
            if l3_entry & PAGE_VALID == 0 { continue; }

            // Free the physical page.
            let phys = PhysicalAddress::new(l3_entry & mask);
            unsafe { allocator.free_page(phys) };
            unsafe { *l3_entry_ptr = 0 };

            // Invalidate TLB entry.
            unsafe {
                core::arch::asm!(
                    "dsb ishst",
                    "tlbi vaae1is, {va}",
                    "dsb ish",
                    "isb",
                    va = in(reg) va >> 12,
                    options(nostack, preserves_flags)
                );
            }
        }
    }

    /// Activate this page table as the kernel TTBR1_EL1 mapping.
    ///
    /// Configures MAIR_EL1 and TCR_EL1 before writing TTBR1_EL1.
    /// After this call the CPU uses this page table for all 0xFFFF… addresses.
    ///
    /// The TTBR0 walk is disabled (EPD0=1) until `enable_user_space()` is called.
    pub fn activate(&self) {
        // -----------------------------------------------------------------------
        // MAIR_EL1 — Memory Attribute Indirection Register
        // Reference: ARM ARM DDI 0487 D17.2.97.
        //
        // Attr0 = 0xFF: Normal WB Non-Transient RA+WA (inner and outer).
        //   Used by PAGE_ATTR_NORMAL pages (kernel code, data, heap).
        // Attr1 = 0x00: Device-nGnRnE.
        //   Used by PAGE_ATTR_DEVICE pages (GIC, UART, framebuffer).
        // -----------------------------------------------------------------------
        let mair: u64 = (0xFF_u64 << 0) // Attr0: Normal WB
                      | (0x00_u64 << 8); // Attr1: Device nGnRnE

        // -----------------------------------------------------------------------
        // TCR_EL1 — Translation Control Register
        // Reference: ARM ARM DDI 0487 D17.2.131.
        //
        // T0SZ = T1SZ = 16  →  48-bit VA for both TTBR0 and TTBR1.
        // TG1 = 0b10         →  4 KiB granule (note: TG1 encoding ≠ TG0 encoding).
        // IRGN1/ORGN1 = 0b01 →  kernel table walk: inner/outer WB RA+WA.
        // SH1 = 0b11         →  inner shareable.
        // EPD0 = 1           →  TTBR0 walk disabled until user process runs.
        // IPS from ID_AA64MMFR0_EL1.PARange (same encoding as PARange).
        // -----------------------------------------------------------------------
        let mmfr0: u64;
        unsafe { core::arch::asm!("mrs {}, id_aa64mmfr0_el1", out(reg) mmfr0) };
        let pa_range = mmfr0 & 0xF;

        let tcr: u64 =
            (16_u64 << 0)    // T0SZ  [5:0]   = 16 → 48-bit TTBR0 range
            | (1_u64 << 7)   // EPD0  [7]     = 1  → TTBR0 walks disabled
            | (16_u64 << 16) // T1SZ  [21:16] = 16 → 48-bit TTBR1 range (kernel)
            | (1_u64 << 24)  // IRGN1 [25:24] = 01 → inner WB RA+WA
            | (1_u64 << 26)  // ORGN1 [27:26] = 01 → outer WB RA+WA
            | (3_u64 << 28)  // SH1   [29:28] = 11 → inner shareable
            | (2_u64 << 30)  // TG1   [31:30] = 10 → 4 KiB (TG1 encoding: 0b10=4K)
            | (pa_range << 32); // IPS [34:32] from PARange

        let root_phys = self.root_phys.as_u64();

        // Write order: MAIR → TCR (ISB) → TTBR1 (ISB) → TLB flush.
        // ISB after TCR ensures new TCR is visible before TTBR1 is written.
        // "dsb sy" after tlbi ensures global TLB invalidation is complete.
        // Reference: ARM ARM DDI 0487 D8.11.
        unsafe {
            core::arch::asm!(
                "msr mair_el1, {mair}",
                "msr tcr_el1,  {tcr}",
                "isb",
                "msr ttbr1_el1, {root}",
                "isb",
                "tlbi vmalle1is", // invalidate all EL1 TLB entries, inner-shareable broadcast
                "dsb sy",         // system-wide barrier
                "isb",
                mair = in(reg) mair,
                tcr  = in(reg) tcr,
                root = in(reg) root_phys,
                options(nostack, preserves_flags)
            );
        }
    }

    /// Enable TTBR0 walks in TCR_EL1 (clears EPD0).
    ///
    /// Call once before the first `eret` to EL0.  Configures TG0 and the
    /// TTBR0 cacheability/shareability fields to match the TTBR1 settings.
    pub fn enable_user_space() {
        let mmfr0: u64;
        unsafe { core::arch::asm!("mrs {}, id_aa64mmfr0_el1", out(reg) mmfr0) };
        let pa_range = mmfr0 & 0xF;

        // EPD0 is absent here → TTBR0 walks enabled.
        // TG0 = 0b00 → 4 KiB (note: TG0 0b00 = 4K, unlike TG1 where 0b10 = 4K).
        let tcr: u64 =
            (16_u64 << 0)    // T0SZ  [5:0]   = 16
            | (1_u64 << 8)   // IRGN0 [9:8]   = 01 → inner WB RA+WA
            | (1_u64 << 10)  // ORGN0 [11:10] = 01 → outer WB RA+WA
            | (3_u64 << 12)  // SH0   [13:12] = 11 → inner shareable
            | (0_u64 << 14)  // TG0   [15:14] = 00 → 4 KiB
            | (16_u64 << 16) // T1SZ  [21:16] = 16
            | (1_u64 << 24)  // IRGN1 [25:24] = 01
            | (1_u64 << 26)  // ORGN1 [27:26] = 01
            | (3_u64 << 28)  // SH1   [29:28] = 11
            | (2_u64 << 30)  // TG1   [31:30] = 10
            | (pa_range << 32);

        unsafe {
            core::arch::asm!(
                "msr tcr_el1, {tcr}",
                "isb",
                "tlbi vmalle1is",
                "dsb ish",
                "isb",
                tcr = in(reg) tcr,
                options(nostack, preserves_flags)
            );
        }
    }

    /// Switch TTBR0_EL1 to a user process page table and flush TLB.
    ///
    /// # Safety
    /// `user_table_phys` must be the physical address of a valid L0 table.
    pub unsafe fn switch_user_table(user_table_phys: PhysicalAddress) {
        let phys = user_table_phys.as_u64();
        core::arch::asm!(
            "msr ttbr0_el1, {phys}",
            "isb",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            phys = in(reg) phys,
            options(nostack, preserves_flags)
        );
    }

    /// Deep-copy a user page table (lower-half TTBR0 range only).
    ///
    /// Allocates fresh intermediate tables and fresh physical pages for every
    /// L3 leaf in `src`.  If any allocation fails, ALL allocations made during
    /// this call are freed before returning `Err` — no leaks.
    ///
    /// # Rollback strategy
    /// We maintain a flat list of all physical pages allocated during the copy.
    /// On failure we free every entry in that list before returning.
    /// The list is stored on the stack using a fixed-capacity inline array —
    /// no heap dependency at this layer.
    ///
    /// Maximum allocation count per deep_copy: 1 (L0) + 256 (L1) + 256*512 (L2)
    /// + 256*512*512 (L3) = impractical to list all on stack.  We use a page-level
    /// approach: allocate one page at a time and track its physical address.
    /// For rollback we walk the already-built destination tree rather than a list.
    pub fn deep_copy_user(
        src: &PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Result<PageTable, MapError> {
        let mask = pa_mask();
        let hhdm = allocator.hhdm_offset();

        let (dst_l0_ptr, dst_l0_phys) =
            create_table(allocator).ok_or(MapError::OutOfPhysicalMemory)?;

        let src_l0 = src.root_ptr();

        // Iterate lower-half L0 entries only (indices 0..256 for 48-bit VA split).
        // The upper half belongs to TTBR1 (kernel) and is not copied.
        for i0 in 0..256usize {
            let src_l0_entry = unsafe { *src_l0.add(i0) };
            if src_l0_entry & PAGE_VALID == 0 {
                continue;
            }

            let src_l1 = entry_to_table_ptr(src_l0_entry, hhdm, mask);

            let (dst_l1_ptr, dst_l1_phys) = match create_table(allocator) {
                Some(t) => t,
                None => {
                    free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                    unsafe { allocator.free_page(dst_l0_phys) };
                    return Err(MapError::OutOfPhysicalMemory);
                }
            };
            unsafe {
                *dst_l0_ptr.add(i0) = dst_l1_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
            }

            for i1 in 0..TABLE_ENTRY_COUNT {
                let src_l1_entry = unsafe { *src_l1.add(i1) };
                if src_l1_entry & PAGE_VALID == 0 {
                    continue;
                }

                let src_l2 = entry_to_table_ptr(src_l1_entry, hhdm, mask);

                let (dst_l2_ptr, dst_l2_phys) = match create_table(allocator) {
                    Some(t) => t,
                    None => {
                        free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                        unsafe { allocator.free_page(dst_l0_phys) };
                        return Err(MapError::OutOfPhysicalMemory);
                    }
                };
                unsafe {
                    *dst_l1_ptr.add(i1) = dst_l2_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                }

                for i2 in 0..TABLE_ENTRY_COUNT {
                    let src_l2_entry = unsafe { *src_l2.add(i2) };
                    if src_l2_entry & PAGE_VALID == 0 {
                        continue;
                    }

                    let src_l3 = entry_to_table_ptr(src_l2_entry, hhdm, mask);

                    let (dst_l3_ptr, dst_l3_phys) = match create_table(allocator) {
                        Some(t) => t,
                        None => {
                            free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                            unsafe { allocator.free_page(dst_l0_phys) };
                            return Err(MapError::OutOfPhysicalMemory);
                        }
                    };
                    unsafe {
                        *dst_l2_ptr.add(i2) =
                            dst_l3_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                    }

                    for i3 in 0..TABLE_ENTRY_COUNT {
                        let src_l3_entry = unsafe { *src_l3.add(i3) };
                        if src_l3_entry & PAGE_VALID == 0 {
                            continue;
                        }

                        // Allocate a fresh page and copy contents.
                        let new_phys = match allocator.alloc() {
                            Some(p) => p,
                            None => {
                                free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                                unsafe { allocator.free_page(dst_l0_phys) };
                                return Err(MapError::OutOfPhysicalMemory);
                            }
                        };

                        let src_page = PhysicalAddress::new(src_l3_entry & mask)
                            .to_virtual(hhdm)
                            .as_ptr::<u8>();
                        let dst_page = new_phys.to_virtual(hhdm).as_ptr::<u8>();

                        unsafe {
                            core::ptr::copy_nonoverlapping(src_page, dst_page, 4096);
                        }

                        // Preserve flags, replace PA.
                        let page_flags = src_l3_entry & !mask;
                        unsafe {
                            *dst_l3_ptr.add(i3) = new_phys.as_u64() | page_flags;
                        }
                    }
                }
            }
        }

        Ok(PageTable {
            root_phys: dst_l0_phys,
            hhdm_offset: hhdm,
        })
    }

    pub fn root_physical(&self) -> PhysicalAddress {
        self.root_phys
    }

    /// Copy-on-Write fork: create a child page table sharing physical pages.
    ///
    /// Instead of allocating new data pages (as `deep_copy_user` does), this
    /// function:
    ///   1. Allocates new L0/L1/L2/L3 table pages for the child (separate
    ///      virtual address spaces require separate walk structures).
    ///   2. For each user R/W leaf (`PAGE_USER_READWRITE`):
    ///      - Marks the parent entry read-only with `PAGE_COW` set.
    ///      - Places the same physical page in the child with R/O + COW flags.
    ///   3. For other leaf entries (genuinely R/O code, signal trampoline):
    ///      - Copies the entry as-is (same physical page, same flags).
    ///
    /// TLB invalidation for parent entries made R/O is performed inline.
    ///
    /// The caller must call `collect_cow_pages()` on both page tables after
    /// this returns to build the `process.cow_pages` maps.
    ///
    /// # Rollback
    /// On any allocation failure, all child tables allocated so far are freed
    /// (same strategy as `deep_copy_user`).  The parent's R/O entries that were
    /// already modified are NOT restored — this is safe: a page made R/O
    /// prematurely will just cause a harmless CoW fault the next time the
    /// parent writes, generating a private copy.
    pub fn cow_copy_user(
        src: &mut PageTable,
        allocator: &mut PhysicalAllocator,
    ) -> Result<PageTable, MapError> {
        let mask = pa_mask();
        let hhdm = allocator.hhdm_offset();

        let (dst_l0_ptr, dst_l0_phys) =
            create_table(allocator).ok_or(MapError::OutOfPhysicalMemory)?;

        let src_l0 = src.root_ptr();

        // Iterate lower-half only (TTBR0 range, indices 0..256).
        for i0 in 0..256usize {
            let src_l0_entry = unsafe { *src_l0.add(i0) };
            if src_l0_entry & PAGE_VALID == 0 {
                continue;
            }

            let src_l1 = entry_to_table_ptr(src_l0_entry, hhdm, mask);

            let (dst_l1_ptr, dst_l1_phys) = match create_table(allocator) {
                Some(t) => t,
                None => {
                    free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                    unsafe { allocator.free_page(dst_l0_phys) };
                    return Err(MapError::OutOfPhysicalMemory);
                }
            };
            unsafe {
                *dst_l0_ptr.add(i0) = dst_l1_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
            }

            for i1 in 0..TABLE_ENTRY_COUNT {
                let src_l1_entry = unsafe { *src_l1.add(i1) };
                if src_l1_entry & PAGE_VALID == 0 {
                    continue;
                }

                let src_l2 = entry_to_table_ptr(src_l1_entry, hhdm, mask);

                let (dst_l2_ptr, dst_l2_phys) = match create_table(allocator) {
                    Some(t) => t,
                    None => {
                        free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                        unsafe { allocator.free_page(dst_l0_phys) };
                        return Err(MapError::OutOfPhysicalMemory);
                    }
                };
                unsafe {
                    *dst_l1_ptr.add(i1) = dst_l2_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                }

                for i2 in 0..TABLE_ENTRY_COUNT {
                    let src_l2_entry = unsafe { *src_l2.add(i2) };
                    if src_l2_entry & PAGE_VALID == 0 {
                        continue;
                    }

                    let src_l3 = entry_to_table_ptr(src_l2_entry, hhdm, mask);

                    let (dst_l3_ptr, dst_l3_phys) = match create_table(allocator) {
                        Some(t) => t,
                        None => {
                            free_partial_copy(dst_l0_ptr, hhdm, mask, allocator);
                            unsafe { allocator.free_page(dst_l0_phys) };
                            return Err(MapError::OutOfPhysicalMemory);
                        }
                    };
                    unsafe {
                        *dst_l2_ptr.add(i2) =
                            dst_l3_phys.as_u64() | PAGE_VALID | PAGE_TABLE;
                    }

                    for i3 in 0..TABLE_ENTRY_COUNT {
                        let src_l3_entry = unsafe { *src_l3.add(i3) };
                        if src_l3_entry & PAGE_VALID == 0 {
                            continue;
                        }

                        // Determine the flags for both parent and child entries.
                        let ap_bits = src_l3_entry & (3 << 6);
                        let (parent_entry, child_entry) =
                            if ap_bits == PAGE_USER_READWRITE {
                                // R/W data page: CoW-share by making both R/O.
                                let cow_entry = (src_l3_entry & !((3 << 6) | PAGE_COW))
                                    | PAGE_USER_READONLY
                                    | PAGE_COW;
                                (cow_entry, cow_entry)
                            } else {
                                // Already R/O (code, trampoline): copy as-is.
                                (src_l3_entry, src_l3_entry)
                            };

                        // Update parent entry in-place.
                        let src_l3_entry_ptr = unsafe { src_l3.add(i3) };
                        unsafe { *src_l3_entry_ptr = parent_entry };

                        // Write child entry.
                        unsafe { *dst_l3_ptr.add(i3) = child_entry };
                    }
                }
            }
        }

        // TLB flush for the parent's entries that were made read-only.
        // A full TTBR0 flush is conservative but correct; targeted VA-by-VA
        // flushing would require walking again.
        // Reference: ARM ARM DDI 0487 D8.11.
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vmalle1is",
                "dsb ish",
                "isb",
                options(nostack, preserves_flags)
            );
        }

        Ok(PageTable {
            root_phys: dst_l0_phys,
            hhdm_offset: hhdm,
        })
    }

    /// Enumerate all CoW-marked leaf entries in the lower half of this page table.
    ///
    /// Calls `callback(page_va, physical_address)` for each entry with the
    /// `PAGE_COW` bit set.  Used to build `process.cow_pages` after a fork.
    ///
    /// Must be called outside of any physical-allocator closure to allow the
    /// callback to use the heap (e.g., insert into BTreeMap).
    pub fn collect_cow_pages(
        &self,
        callback: &mut dyn FnMut(u64, PhysicalAddress),
    ) {
        let mask = pa_mask();
        let hhdm = self.hhdm_offset;
        let l0 = self.root_ptr();

        for i0 in 0..256usize {
            let l0_entry = unsafe { *l0.add(i0) };
            if l0_entry & PAGE_VALID == 0 { continue; }
            let l1 = entry_to_table_ptr(l0_entry, hhdm, mask);

            for i1 in 0..TABLE_ENTRY_COUNT {
                let l1_entry = unsafe { *l1.add(i1) };
                if l1_entry & PAGE_VALID == 0 { continue; }
                let l2 = entry_to_table_ptr(l1_entry, hhdm, mask);

                for i2 in 0..TABLE_ENTRY_COUNT {
                    let l2_entry = unsafe { *l2.add(i2) };
                    if l2_entry & PAGE_VALID == 0 { continue; }
                    let l3 = entry_to_table_ptr(l2_entry, hhdm, mask);

                    for i3 in 0..TABLE_ENTRY_COUNT {
                        let l3_entry = unsafe { *l3.add(i3) };
                        if l3_entry & PAGE_VALID == 0 { continue; }
                        if l3_entry & PAGE_COW == 0 { continue; }

                        // Reconstruct the virtual address from table indices.
                        let va = ((i0 as u64) << 39)
                            | ((i1 as u64) << 30)
                            | ((i2 as u64) << 21)
                            | ((i3 as u64) << 12);
                        let phys = PhysicalAddress::new(l3_entry & mask);
                        callback(va, phys);
                    }
                }
            }
        }
    }
}

/// Recursively free all physical pages referenced by a partially-constructed
/// destination L0 table.  Used for rollback on deep_copy_user failure.
///
/// This only frees L1/L2/L3 *tables* and leaf *pages* — the L0 itself is
/// freed by the caller.
fn free_partial_copy(
    l0_ptr: *mut u64,
    hhdm: u64,
    mask: u64,
    allocator: &mut PhysicalAllocator,
) {
    for i0 in 0..256usize {
        let l0_entry = unsafe { *l0_ptr.add(i0) };
        if l0_entry & PAGE_VALID == 0 { continue; }

        let l1_phys = PhysicalAddress::new(l0_entry & mask);
        let l1_ptr = l1_phys.to_virtual(hhdm).as_ptr::<u64>();

        for i1 in 0..TABLE_ENTRY_COUNT {
            let l1_entry = unsafe { *l1_ptr.add(i1) };
            if l1_entry & PAGE_VALID == 0 { continue; }

            let l2_phys = PhysicalAddress::new(l1_entry & mask);
            let l2_ptr = l2_phys.to_virtual(hhdm).as_ptr::<u64>();

            for i2 in 0..TABLE_ENTRY_COUNT {
                let l2_entry = unsafe { *l2_ptr.add(i2) };
                if l2_entry & PAGE_VALID == 0 { continue; }

                let l3_phys = PhysicalAddress::new(l2_entry & mask);
                let l3_ptr = l3_phys.to_virtual(hhdm).as_ptr::<u64>();

                for i3 in 0..TABLE_ENTRY_COUNT {
                    let l3_entry = unsafe { *l3_ptr.add(i3) };
                    if l3_entry & PAGE_VALID == 0 { continue; }
                    let page_phys = PhysicalAddress::new(l3_entry & mask);
                    unsafe { allocator.free_page(page_phys) };
                }

                unsafe { allocator.free_page(l3_phys) };
            }

            unsafe { allocator.free_page(l2_phys) };
        }

        unsafe { allocator.free_page(l1_phys) };
    }
}

/// Allocate `page_count` physical pages and map them contiguously starting at
/// `virtual_address` in the given page table with `flags`.
///
/// Returns `Err` if any physical allocation or mapping fails.  On failure,
/// any pages already mapped by *this call* are unmapped and freed.
pub fn kernel_virtual_memory_alloc(
    page_table: &mut PageTable,
    virtual_address: VirtualAddress,
    page_count: usize,
    flags: u64,
    page_size: u64,
    allocator: &mut PhysicalAllocator,
) -> Result<(), MapError> {
    let mut mapped = 0usize;

    for i in 0..page_count {
        let virt = VirtualAddress::new(virtual_address.as_u64() + i as u64 * page_size);
        let phys = allocator.alloc().ok_or_else(|| {
            // Roll back: unmap pages already mapped in this call.
            page_table.unmap_range(virtual_address, mapped, page_size, allocator);
            MapError::OutOfPhysicalMemory
        })?;

        page_table.map(virt, phys, flags, allocator).map_err(|e| {
            // map() failed — free the page we just allocated and roll back the rest.
            unsafe { allocator.free_page(phys) };
            page_table.unmap_range(virtual_address, mapped, page_size, allocator);
            e
        })?;

        mapped += 1;
    }

    Ok(())
}
