// memory/physical.rs — Physical page allocator.
//
// Algorithm: free list (LIFO) threaded through the HHDM.
//
// Each free page stores a pointer to the next free page at the beginning of
// its own HHDM-mapped memory.  This keeps the free list accessible regardless
// of which TTBR1 page table is active, as long as the HHDM is mapped.
//
// Compared to the C implementation:
// - State is encapsulated in `PhysicalAllocator`; no module-level statics.
// - `alloc()` returns `Option<PhysicalAddress>` — OOM is explicit, not a NULL cast.
// - `init()` uses PFN_UP/PFN_DOWN rounding — the same conservative approach as
//   Linux mm/memblock.c — to skip partial pages on malformed firmware tables.
// - The HHDM offset is a constructor argument, not an implicit global.
//
// Reference: ARM ARM DDI 0487, B4 "Translation Table Walk".
// Reference: Linux kernel mm/page_alloc.c (free list threading concept).

use super::address::{align_down, align_up, PhysicalAddress, VirtualAddress};
use crate::limine::{MemmapResponse, MEMMAP_USABLE};

// AArch64 supports 4 KiB, 16 KiB, and 64 KiB granules.
// We detect the active granule at runtime from TCR_EL1.TG1 in `read_page_size()`.
// However, at the time physical_memory_init() runs, the MMU is still configured
// by Limine with 4 KiB pages, so we use the Limine-set page size for init.
// After we activate our own page table, `PAGE_SIZE` should match TCR_EL1.TG1.

/// Read the active kernel-side page size from TCR_EL1.TG1.
///
/// TCR_EL1.TG1 [31:30]:
///   0b01 = 16 KiB
///   0b10 =  4 KiB  (AArch64 default; Limine sets this)
///   0b11 = 64 KiB
///
/// Reference: ARM ARM DDI 0487, D13.2.120 TCR_EL1.
pub fn read_page_size() -> u64 {
    let tcr: u64;
    unsafe { core::arch::asm!("mrs {}, tcr_el1", out(reg) tcr) };
    match (tcr >> 30) & 0b11 {
        0b01 => 16 * 1024,
        0b11 => 64 * 1024,
        _ => 4 * 1024, // 0b10 is 4 KiB; treat unknown as 4 KiB
    }
}

// Node threaded through each free page (stored at the page's HHDM virtual address).
struct FreePage {
    next: Option<PhysicalAddress>,
}

pub struct PhysicalAllocator {
    free_list_head: Option<PhysicalAddress>,
    free_page_count: usize,
    total_bytes: u64,
    usable_bytes: u64,
    hhdm_offset: u64,
}

impl PhysicalAllocator {
    pub const fn new(hhdm_offset: u64) -> Self {
        Self {
            free_list_head: None,
            free_page_count: 0,
            total_bytes: 0,
            usable_bytes: 0,
            hhdm_offset,
        }
    }

    /// Populate the free list from the Limine memory map.
    ///
    /// Pass 1: compute `total_bytes` (ceiling of physical address space) and
    ///         `usable_bytes` (sum of USABLE entry lengths).
    ///
    /// Pass 2: add all USABLE pages to the free list using PFN_UP/PFN_DOWN
    ///         rounding so we never add a page that is only partially usable.
    pub fn init(&mut self, memmap: &MemmapResponse) {
        let page_size = read_page_size();

        // Pass 1: metrics.
        for i in 0..memmap.entry_count as usize {
            let entry = unsafe { &**memmap.entries.add(i) };
            let entry_end = entry.base + entry.length;
            if entry_end > self.total_bytes {
                self.total_bytes = entry_end;
            }
            if entry.entry_type == MEMMAP_USABLE {
                self.usable_bytes += entry.length;
            }
        }

        // Pass 2: build free list.
        for i in 0..memmap.entry_count as usize {
            let entry = unsafe { &**memmap.entries.add(i) };
            if entry.entry_type != MEMMAP_USABLE {
                continue;
            }

            // PFN_UP: round base up; PFN_DOWN: round end down.
            let base = align_up(entry.base, page_size);
            let end = align_down(entry.base + entry.length, page_size);

            if end <= base {
                continue; // zero-length or unaligned region
            }

            let mut address = base;
            while address < end {
                // Safety: HHDM maps all usable physical memory.
                unsafe { self.free_page(PhysicalAddress::new(address)) };
                address += page_size;
            }
        }
    }

    /// Allocate one physical page. Returns `None` if no pages are available (OOM).
    ///
    /// The returned `PhysicalAddress` is suitable for page table entries.
    /// Call `PhysicalAddress::to_virtual(hhdm_offset)` to access the page contents.
    pub fn alloc(&mut self) -> Option<PhysicalAddress> {
        let head = self.free_list_head?;

        // Read the next pointer from the node stored at the HHDM virtual address.
        let node_virt = head.to_virtual(self.hhdm_offset);
        let node = unsafe { &*(node_virt.as_ptr::<FreePage>()) };
        self.free_list_head = node.next;
        self.free_page_count -= 1;

        Some(head)
    }

    /// Return a physical page to the free list.
    ///
    /// # Safety
    /// `page` must be a valid physical page address that was previously returned
    /// by `alloc()` or is a usable page from the memory map.  The page must not
    /// be freed more than once (no double-free detection at this layer — that is
    /// the caller's responsibility, as at the physical layer we have no metadata).
    pub unsafe fn free_page(&mut self, page: PhysicalAddress) {
        let node_virt: VirtualAddress = page.to_virtual(self.hhdm_offset);
        let node = &mut *(node_virt.as_ptr::<FreePage>());
        node.next = self.free_list_head;
        self.free_list_head = Some(page);
        self.free_page_count += 1;
    }

    pub fn free_page_count(&self) -> usize {
        self.free_page_count
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn usable_bytes(&self) -> u64 {
        self.usable_bytes
    }

    pub fn hhdm_offset(&self) -> u64 {
        self.hhdm_offset
    }
}
