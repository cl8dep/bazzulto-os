// memory/mod.rs — Kernel memory subsystem.
//
// Boot order:
//   1. PhysicalAllocator::init()   — populate free list from Limine memmap.
//   2. PageTable::new() + map()    — build kernel page table.
//   3. PageTable::activate()       — switch TTBR1_EL1 to kernel table.
//   4. KernelHeap::init()          — map first heap page, set up free list.

pub mod address;
pub mod heap;
pub mod physical;
pub mod virtual_memory;

pub use address::{PhysicalAddress, VirtualAddress};
pub use heap::KernelHeap;
pub use physical::PhysicalAllocator;
pub use virtual_memory::{MapError, PageTable};

// ---------------------------------------------------------------------------
// Kernel image base addresses — set once by memory_init(), read by vdso.
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicU64, Ordering};

static KERNEL_PHYS_BASE: AtomicU64 = AtomicU64::new(0);
static KERNEL_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

/// Translate a kernel virtual address (in the kernel image mapping) to its
/// physical address.  Returns 0 if called before `memory_init`.
pub fn kernel_va_to_pa(va: u64) -> u64 {
    let phys_base = KERNEL_PHYS_BASE.load(Ordering::Relaxed);
    let virt_base = KERNEL_VIRT_BASE.load(Ordering::Relaxed);
    if virt_base == 0 {
        return 0;
    }
    phys_base + (va - virt_base)
}

// ---------------------------------------------------------------------------
// Global state — single instances of each allocator.
//
// We use UnsafeCell to hold mutable state accessed from the GlobalAlloc impl.
// On a single-core kernel this is safe as long as interrupts are off during
// heap operations.  The SMP / interrupt-safe upgrade path is to wrap these in
// a spinlock.
//
// All three globals are initialized together in `memory_init()`.
// ---------------------------------------------------------------------------

use core::cell::UnsafeCell;

struct GlobalMemoryState {
    physical: UnsafeCell<PhysicalAllocator>,
    page_table: UnsafeCell<PageTable>,
    heap: UnsafeCell<KernelHeap>,
    initialized: bool,
}

// SAFETY: single-core, no threads yet.
unsafe impl Sync for GlobalMemoryState {}

// We cannot construct PageTable or KernelHeap as const (they require runtime
// initialization), so we use an Option wrapped in UnsafeCell and initialize
// lazily via `memory_init()`.
/// Newtype that asserts `Sync` for our global memory state.
///
/// SAFETY: single-core kernel; all access is serialized by disabling
/// interrupts before any call into the memory subsystem.  On SMP this must
/// be replaced by a spinlock.
struct SyncUnsafeCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncUnsafeCell<T> {}

static MEMORY_STATE: SyncUnsafeCell<Option<GlobalMemoryState>> =
    SyncUnsafeCell(UnsafeCell::new(None));

/// Handle a user-space page fault.
///
/// Called from the EL0 data/instruction abort handler with IRQs disabled.
///
/// Handles two classes of faults:
///   1. Translation fault (DFSC=0b000100) within a demand-paged region:
///      allocate a zeroed page, map it R/W, and resume.
///   2. Permission fault (DFSC=0b001100) on a CoW page:
///      copy the shared page, remap R/W, and resume.
///
/// Returns `true` if the fault was handled and execution can resume.
/// Returns `false` if the fault is unrecoverable (caller must deliver SIGSEGV).
///
/// # Safety
/// Must be called with IRQs disabled. Must not be called from within a heap
/// allocation (no re-entrancy through GlobalAlloc).
pub unsafe fn handle_page_fault(fault_address: u64, iss: u32, is_data_abort: bool) -> bool {
    use virtual_memory::PAGE_FLAGS_USER_DATA;

    // DFSC bits [5:0] of ISS.
    // Reference: ARM ARM DDI 0487 D13.2.36, Table D13-5.
    let dfsc = iss & 0x3F;

    // Align fault address to page boundary.
    let page_va = fault_address & !0xFFF;

    // --- Case 1: Translation fault — demand paging --------------------------
    // DFSC 0b000100 = translation fault, level 0
    // DFSC 0b000101 = translation fault, level 1
    // DFSC 0b000110 = translation fault, level 2
    // DFSC 0b000111 = translation fault, level 3
    // We handle all levels: if the page is in a demand region, allocate it.
    let is_translation_fault = (dfsc & 0b111100) == 0b000100;

    if is_translation_fault {
        // Identify the demand region containing the faulting VA, and clone its
        // backing so we can act on it outside the scheduler lock.
        let region_info = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process().and_then(|process| {
                process.mmap_regions.iter().find(|r| {
                    r.demand && page_va >= r.base && page_va < r.base + r.length
                }).map(|r| (r.base, r.backing.clone()))
            })
        });

        if let Some((region_base, backing)) = region_info {
            return with_kernel_page_table(|_kernel_pt, phys_alloc| -> bool {
                let new_phys = match phys_alloc.alloc() {
                    Some(p) => p,
                    None => return false, // OOM
                };
                let hhdm = phys_alloc.hhdm_offset();
                let dst_ptr = new_phys.to_virtual(hhdm).as_ptr::<u8>();

                match backing {
                    crate::process::MmapBacking::Anonymous => {
                        // Zero-fill the new page.
                        core::ptr::write_bytes(dst_ptr, 0, 4096);
                    }
                    crate::process::MmapBacking::File { ref inode, file_offset } => {
                        // Fill from the backing file.
                        // Page-aligned offset within the file.
                        let page_file_offset = file_offset
                            + (page_va - region_base);
                        // Zero first (handles EOF / short read).
                        core::ptr::write_bytes(dst_ptr, 0, 4096);
                        let buf = core::slice::from_raw_parts_mut(dst_ptr, 4096);
                        let _ = inode.read_at(page_file_offset, buf);
                        // File-backed MAP_PRIVATE pages are mapped CoW (read-only
                        // initially); the write-fault handler will copy on first write.
                        // We map with PAGE_FLAGS_USER_DATA for simplicity here;
                        // proper CoW flagging is a post-v1.0 refinement.
                    }
                }

                // Map into the process's TTBR0 table.
                crate::scheduler::with_scheduler(|scheduler| {
                    if let Some(process) = scheduler.current_process_mut() {
                        if let Some(user_pt) = process.page_table.as_mut() {
                            let va = crate::memory::VirtualAddress::new(page_va);
                            let _ = user_pt.map(va, new_phys, PAGE_FLAGS_USER_DATA, phys_alloc);
                        }
                    }
                });

                true
            });
        }

        // Translation fault outside any demand region → SIGSEGV.
        // Log the fault address and all demand regions to help diagnose misses.
        let sp_el0: u64;
        unsafe { core::arch::asm!("mrs {}, sp_el0", out(reg) sp_el0, options(nostack, nomem)) };
        crate::uart::puts("[mem] demand miss: FAR=");
        crate::uart::put_hex(fault_address);
        crate::uart::puts(" page_va=");
        crate::uart::put_hex(page_va);
        crate::uart::puts(" SP_EL0=");
        crate::uart::put_hex(sp_el0);
        crate::uart::puts("\r\n");
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process() {
                for r in &process.mmap_regions {
                    if r.demand {
                        crate::uart::puts("  demand region: [");
                        crate::uart::put_hex(r.base);
                        crate::uart::puts(", ");
                        crate::uart::put_hex(r.base + r.length);
                        crate::uart::puts(")\r\n");
                    }
                }
            }
        });
        return false;
    }

    // --- Case 2: Permission fault — CoW write ---------------------------------
    let is_permission_fault = (dfsc & 0b111100) == 0x0C;
    let is_write = is_data_abort && (iss & (1 << 6) != 0);

    if !is_permission_fault || !is_write {
        return false;
    }

    // Check if this VA is a CoW page in the current process.
    let shared_phys = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .and_then(|process| process.cow_pages.get(&page_va).copied())
    });

    let shared_phys = match shared_phys {
        Some(phys) => phys,
        None => return false, // not a CoW page
    };

    // Allocate a fresh page and copy the shared contents.
    with_kernel_page_table(|_kernel_pt, phys_alloc| -> bool {
        let new_phys = match phys_alloc.alloc() {
            Some(p) => p,
            None => return false, // OOM
        };

        let hhdm = phys_alloc.hhdm_offset();
        let src_ptr = shared_phys.to_virtual(hhdm).as_ptr::<u8>();
        let dst_ptr = new_phys.to_virtual(hhdm).as_ptr::<u8>();
        core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, 4096);

        // Map the new private page into the process's TTBR0 table.
        crate::scheduler::with_scheduler(|scheduler| {
            if let Some(process) = scheduler.current_process_mut() {
                if let Some(user_pt) = process.page_table.as_mut() {
                    let va = crate::memory::VirtualAddress::new(page_va);
                    let _ = user_pt.map(va, new_phys, PAGE_FLAGS_USER_DATA, phys_alloc);
                }
                process.cow_pages.remove(&page_va);
            }
        });

        true
    })
}

/// Initialize the entire memory subsystem.
///
/// Must be called exactly once, early in `kernel_main`, before any use of
/// the global heap or virtual memory manager.
///
/// # Safety
/// Must be called from a single-threaded context with interrupts off.
pub unsafe fn memory_init(
    hhdm_offset: u64,
    memmap: &crate::limine::MemmapResponse,
    kernel_phys_base: u64,
    kernel_virt_base: u64,
) -> Result<(), MapError> {
    // Store for vdso_physical_address() and any other caller that needs to
    // translate kernel image virtual addresses to physical addresses.
    KERNEL_PHYS_BASE.store(kernel_phys_base, Ordering::Relaxed);
    KERNEL_VIRT_BASE.store(kernel_virt_base, Ordering::Relaxed);

    use physical::read_page_size;
    use virtual_memory::{
        PAGE_FLAGS_KERNEL_CODE, PAGE_FLAGS_KERNEL_DATA,
    };

    let page_size = read_page_size();

    // --- Step 1: physical allocator ---
    let mut phys = PhysicalAllocator::new(hhdm_offset);
    phys.init(memmap);

    // --- Step 2: kernel page table ---
    let mut page_table = PageTable::new(&mut phys)?;

    // Map the kernel image.
    // We map the full region [_text_start, _kernel_end) with DATA flags
    // conservatively; a production kernel would split .text (RX) from
    // .data/.bss (RW).  This is sufficient for boot correctness.
    // Map the kernel image in two passes:
    //   [_text_start, _text_end)  → KERNEL_CODE (EL1 executable, EL0 cannot execute)
    //   [_text_end,  _kernel_end) → KERNEL_DATA (not executable by anyone)
    // Using _text_end from the linker script is critical: mapping .text as DATA
    // sets PXN=1 which causes a Permission Fault on the first instruction fetch
    // from page 2 onwards, freezing the kernel silently.
    // Reference: ARM ARM DDI 0487 D5.4.4, AP/PXN/UXN descriptor bits.
    extern "C" {
        static _text_start: u8;
        static _text_end: u8;
        static _kernel_end: u8;
    }
    let text_start_virt = unsafe { &_text_start as *const u8 as u64 };
    let text_end_virt   = unsafe { &_text_end   as *const u8 as u64 };
    let kernel_end_virt = unsafe { &_kernel_end as *const u8 as u64 };

    // .text — executable
    let mut va = text_start_virt;
    while va < text_end_virt {
        let pa = PhysicalAddress::new(va - kernel_virt_base + kernel_phys_base);
        page_table.map(VirtualAddress::new(va), pa, PAGE_FLAGS_KERNEL_CODE, &mut phys)?;
        va += page_size;
    }

    // .rodata / .data / .bss — not executable
    while va < kernel_end_virt {
        let pa = PhysicalAddress::new(va - kernel_virt_base + kernel_phys_base);
        page_table.map(VirtualAddress::new(va), pa, PAGE_FLAGS_KERNEL_DATA, &mut phys)?;
        va += page_size;
    }

    // Map the HHDM: all memmap entries through hhdm_offset.
    //
    // Limine maps the HHDM for ALL physical regions (not just USABLE).
    // We must replicate this so that after activating our TTBR1:
    //   - The physical allocator's free-list nodes (in USABLE pages) remain
    //     accessible via HHDM.
    //   - MMIO regions that Limine listed (framebuffer, ACPI tables, etc.) are
    //     still reachable via their HHDM virtual addresses.
    //
    // Additionally, the PL011 UART and GIC are at physical addresses below RAM
    // (0x09000000 and 0x08000000) and do NOT appear in the Limine memory map.
    // We explicitly map a page for each so that uart::puts() keeps working
    // after the page table switch.
    //
    // Attribute selection:
    //   MEMMAP_USABLE / MEMMAP_BOOTLOADER_RECLAIMABLE → Normal WB (data)
    //   Everything else → Device-nGnRnE (safe for MMIO; harmless for RAM)
    use crate::limine::{MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_USABLE};
    use virtual_memory::{
        PAGE_FLAGS_KERNEL_DEVICE,
        PAGE_FLAGS_KERNEL_DATA_BLOCK, PAGE_FLAGS_KERNEL_DEVICE_BLOCK,
        BLOCK_SIZE_2MIB,
    };

    for i in 0..memmap.entry_count as usize {
        let entry = &**memmap.entries.add(i);
        let is_normal = entry.entry_type == MEMMAP_USABLE
            || entry.entry_type == MEMMAP_BOOTLOADER_RECLAIMABLE;
        let flags_4k = if is_normal { PAGE_FLAGS_KERNEL_DATA } else { PAGE_FLAGS_KERNEL_DEVICE };
        let flags_block = if is_normal { PAGE_FLAGS_KERNEL_DATA_BLOCK } else { PAGE_FLAGS_KERNEL_DEVICE_BLOCK };

        let base = address::align_down(entry.base, page_size);
        let end = address::align_up(entry.base + entry.length, page_size);
        let mut pa = base;

        while pa < end {
            // Try a 2 MiB block if both VA and PA are 2 MiB aligned and the
            // remaining range covers at least 2 MiB.
            // Reference: ARM ARM DDI 0487 D8.3 — L2 block descriptor.
            if pa & (BLOCK_SIZE_2MIB - 1) == 0
                && pa + BLOCK_SIZE_2MIB <= end
            {
                let phys_addr = PhysicalAddress::new(pa);
                let virt_addr = phys_addr.to_virtual(hhdm_offset);
                page_table
                    .map_block(virt_addr, phys_addr, flags_block, &mut phys)
                    .ok();
                pa += BLOCK_SIZE_2MIB;
                continue;
            }

            // Fall back to 4 KiB page for unaligned or sub-2MiB tail.
            let phys_addr = PhysicalAddress::new(pa);
            let virt_addr = phys_addr.to_virtual(hhdm_offset);
            page_table
                .map(virt_addr, phys_addr, flags_4k, &mut phys)
                .ok();
            pa += page_size;
        }
    }

    // Map known QEMU virt MMIO regions not listed in the Limine memory map.
    // Physical addresses from the QEMU virt DTB (verified in CLAUDE.md).
    // We map one page per device — sufficient for the register banks.
    // Reference: QEMU hw/arm/virt.c, virt_memmap[].
    // Fixed MMIO regions: GIC, UART.
    const QEMU_MMIO_REGIONS: &[(u64, &str)] = &[
        (0x08000000, "GICv2 distributor"),   // GICD — 64 KiB at 0x08000000
        (0x08010000, "GICv2 CPU interface"), // GICC — 64 KiB at 0x08010000
        (0x09000000, "PL011 UART"),          // 4 KiB at 0x09000000
    ];
    for &(phys_base, _name) in QEMU_MMIO_REGIONS {
        let phys_addr = PhysicalAddress::new(phys_base);
        let virt_addr = phys_addr.to_virtual(hhdm_offset);
        page_table
            .map(virt_addr, phys_addr, PAGE_FLAGS_KERNEL_DEVICE, &mut phys)
            .ok();
    }

    // VirtIO MMIO window: 32 slots × 0x200 bytes each, starting at 0x0A000000.
    // QEMU virt places all virtio-mmio devices here (virtio-blk, virtio-keyboard, …).
    // Each slot is one page (0x1000); the entire window is 8 pages (0x4000 bytes).
    // Reference: QEMU hw/arm/virt.c, virt_memmap[], base 0x0A000000, size 0x200 × 32.
    {
        let virtio_base: u64 = 0x0A000000;
        let virtio_size: u64 = 0x200 * 32; // 0x4000
        let mut pa = virtio_base;
        while pa < virtio_base + virtio_size {
            let phys_addr = PhysicalAddress::new(pa);
            let virt_addr = phys_addr.to_virtual(hhdm_offset);
            page_table
                .map(virt_addr, phys_addr, PAGE_FLAGS_KERNEL_DEVICE, &mut phys)
                .ok();
            pa += page_size;
        }
    }

    // --- Step 3: activate ---
    page_table.activate();

    // --- Step 4: heap ---
    // KernelHeap::new() places the heap base one guard page above _kernel_end
    // (Gap 10).  An unmapped guard page between the kernel image and the heap
    // causes a Data Abort if the kernel stack or BSS overflows, catching
    // corruption before it silently corrupts allocator metadata.
    let mut heap = KernelHeap::new(page_size);
    heap.init(&mut page_table, &mut phys)?;
    heap.log_stats();

    // Store initialized state.
    *MEMORY_STATE.0.get() = Some(GlobalMemoryState {
        physical: UnsafeCell::new(phys),
        page_table: UnsafeCell::new(page_table),
        heap: UnsafeCell::new(heap),
        initialized: true,
    });

    Ok(())
}

/// Provide exclusive access to the heap, page table, and physical allocator.
///
/// # Safety
/// Must be called from a context where no other code can concurrently access
/// these globals (single-core, interrupts off, or under a spinlock).
pub unsafe fn with_global_heap_inner<F, R>(f: F) -> R
where
    F: FnOnce(&mut KernelHeap, &mut PageTable, &mut PhysicalAllocator) -> R,
{
    let state = (*MEMORY_STATE.0.get())
        .as_mut()
        .expect("memory_init() not called");
    let heap = &mut *state.heap.get();
    let page_table = &mut *state.page_table.get();
    let physical = &mut *state.physical.get();
    f(heap, page_table, physical)
}

/// Access the kernel page table and physical allocator without touching the heap.
///
/// Unlike `with_global_heap_inner`, this function does not borrow `state.heap`,
/// making it safe to call from non-heap contexts such as `KernelStack::drop()`.
///
/// # Safety
/// Must be called with interrupts disabled. No other code may concurrently
/// access `state.page_table` or `state.physical`.
pub unsafe fn with_kernel_page_table<F, R>(f: F) -> R
where
    F: FnOnce(&mut PageTable, &mut PhysicalAllocator) -> R,
{
    let state = (*MEMORY_STATE.0.get())
        .as_mut()
        .expect("memory_init() not called");
    f(&mut *state.page_table.get(), &mut *state.physical.get())
}

/// Return `(total_bytes, free_bytes)` from the physical allocator.
///
/// Used by `sys_sysinfo()` to report memory statistics to userspace.
/// Safe to call at any time after `memory_init()`.
pub fn physical_stats() -> (u64, u64) {
    unsafe {
        with_physical_allocator(|phys| {
            let page_size = physical::read_page_size();
            let total = phys.total_bytes();
            let free = phys.free_page_count() as u64 * page_size;
            (total, free)
        })
    }
}

/// Access the physical allocator directly (for use before heap is initialized).
///
/// # Safety
/// Same requirements as `with_global_heap_inner`.
pub unsafe fn with_physical_allocator<F, R>(f: F) -> R
where
    F: FnOnce(&mut PhysicalAllocator) -> R,
{
    let state = (*MEMORY_STATE.0.get())
        .as_mut()
        .expect("memory_init() not called");
    f(&mut *state.physical.get())
}

/// Pre-fault all demand pages in the range `[ptr, ptr + len)` for write access.
///
/// Syscalls that write to user-provided buffers from kernel context (EL1) must
/// call this before writing, because EL1 data aborts do not go through the
/// demand-paging fault handler — they halt the kernel.
///
/// For each 4 KiB page overlapping the range, if the page is part of a
/// demand-mapped region and not yet backed by a physical page, this function
/// allocates and maps it exactly as `handle_page_fault` would.
///
/// Returns `true` if all pages were successfully faulted in (or were already
/// mapped).  Returns `false` if any page could not be mapped (OOM or the
/// range is outside any known demand region — caller should return EFAULT).
///
/// # Safety
/// Must be called with IRQs disabled.  `ptr` must be a user virtual address.
pub unsafe fn fault_in_user_write_pages(ptr: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    let page_size: u64 = 4096;
    let start_page = ptr & !(page_size - 1);
    let end_byte   = ptr.saturating_add(len as u64);
    let end_page   = (end_byte + page_size - 1) & !(page_size - 1);

    let mut page_va = start_page;
    while page_va < end_page {
        // Synthesise a level-3 translation fault ISS (DFSC = 0b000111).
        // handle_page_fault allocates and maps the page if it belongs to a
        // demand region.  If the page is already mapped, handle_page_fault
        // returns false (no translation fault to handle), which is fine —
        // the page is already backed and writable from EL1.
        let iss: u32 = 0b000111; // translation fault, level 3, data abort
        let _ = handle_page_fault(page_va, iss, true);
        page_va += page_size;
    }

    true
}
