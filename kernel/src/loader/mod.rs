// loader/mod.rs — ELF64 loader for AArch64 user processes.
//
// Parses ELF64 headers and PT_LOAD segments, maps them into a fresh user
// page table, sets up a user stack, and returns the entry point and initial SP.
//
// ASLR:
//   The load base offset is randomised using the AArch64 RNDR hardware RNG
//   (available since ARMv8.5-A).  If RNDR is not available (indicated by
//   failure — RNDR sets NZCV.C to 0 on failure), we fall back to CNTPCT_EL0.
//
// Reference:
//   ELF-64 Object File Format v1.5 (SCO).
//   ARM ARM DDI 0487, §C6.2.182 RNDR.
//   Linux arch/arm64/mm/mmap.c (ASLR implementation).

extern crate alloc;

use alloc::boxed::Box;

use crate::memory::virtual_memory::{
    PageTable, MapError, PAGE_FLAGS_USER_CODE, PAGE_FLAGS_USER_DATA,
};
use crate::memory::address::{PhysicalAddress, VirtualAddress};
use crate::memory::physical::PhysicalAllocator;
use crate::process::SIGNAL_TRAMPOLINE_VA;
use crate::process::SIGNAL_TRAMPOLINE_INSTRUCTION;

// ---------------------------------------------------------------------------
// ELF64 constants
// ---------------------------------------------------------------------------

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
const ELF_TYPE_EXECUTABLE: u16 = 2;
/// ET_DYN — used for PIE executables (position-independent, load base is 0).
/// Reference: ELF-64 Object File Format v1.5, §4.
const ELF_TYPE_SHARED: u16 = 3;
const ELF_MACHINE_AARCH64: u16 = 183;

/// ELF program header type: loadable segment.
const PT_LOAD: u32 = 1;

/// ELF segment flag: execute permission.
const PF_X: u32 = 0x1;
/// ELF segment flag: write permission.
const PF_W: u32 = 0x2;

// ---------------------------------------------------------------------------
// ELF64 header — §4 of the ELF-64 spec.
// ---------------------------------------------------------------------------

#[repr(C)]
struct Elf64Header {
    e_ident:     [u8; 16],
    e_type:      u16,
    e_machine:   u16,
    e_version:   u32,
    e_entry:     u64,
    e_phoff:     u64,
    e_shoff:     u64,
    e_flags:     u32,
    e_ehsize:    u16,
    e_phentsize: u16,
    e_phnum:     u16,
    e_shentsize: u16,
    e_shnum:     u16,
    e_shstrndx:  u16,
}

// ---------------------------------------------------------------------------
// ELF64 program header — §5 of the ELF-64 spec.
// ---------------------------------------------------------------------------

#[repr(C)]
struct Elf64ProgramHeader {
    p_type:   u32,
    p_flags:  u32,
    p_offset: u64,
    p_vaddr:  u64,
    p_paddr:  u64,
    p_filesz: u64,
    p_memsz:  u64,
    p_align:  u64,
}

// ---------------------------------------------------------------------------
// LoaderError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderError {
    /// The binary does not start with the ELF magic bytes.
    NotAnElf,
    /// ELF class, data encoding, or machine type is incompatible.
    UnsupportedFormat,
    /// ELF type is not ET_EXEC (we do not support shared objects here).
    NotExecutable,
    /// A PT_LOAD segment VAddr is not page-aligned.
    UnalignedSegment,
    /// Out of physical memory.
    OutOfPhysicalMemory,
    /// Page table mapping failed.
    MappingFailed(MapError),
    /// The ELF header or a program header is out of bounds in the data slice.
    Truncated,
    /// The binary was not found in the VFS or ramfs.
    NotFound,
}

impl From<MapError> for LoaderError {
    fn from(error: MapError) -> Self {
        LoaderError::MappingFailed(error)
    }
}

// ---------------------------------------------------------------------------
// ASLR random number
// ---------------------------------------------------------------------------

/// Collect entropy from multiple hardware sources for ASLR.
///
/// Combines:
///   - CNTPCT_EL0: physical counter (nanosecond precision in real hardware,
///     microsecond in QEMU — still varies per call).
///   - CNTFRQ_EL0: counter frequency (constant, adds bias mixing).
///   - Physical address of the kernel page table root: varies per boot due
///     to physical memory layout randomness from Limine.
///   - A per-call counter to ensure each invocation yields a different value
///     even if CNTPCT is momentarily the same.
///
/// Mixed with a simple Xorshift64 PRNG to spread bits.
///
/// NOT cryptographically secure — sufficient for OS-level ASLR which is a
/// probabilistic mitigation, not a cryptographic guarantee.
///
/// Reference: Linux lib/random32.c (Xorshift PRNG); arm64 ASLR uses
/// getrandom() seeded from multiple hardware sources (arch/arm64/mm/mmap.c).
fn read_aslr_entropy() -> u64 {
    // CNTPCT_EL0: physical timer counter, varies each call.
    // Reference: ARM ARM DDI 0487 §D14.8.22.
    let cntpct: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) cntpct, options(nostack, nomem)) };

    // CNTFRQ_EL0: counter frequency register (constant, ~62.5 MHz in QEMU).
    // Reference: ARM ARM DDI 0487 §D14.8.18.
    let cntfrq: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) cntfrq, options(nostack, nomem)) };

    // Physical address of the kernel page table root from TTBR1_EL1.
    // This reflects the physical memory layout assigned by Limine.
    // Reference: ARM ARM DDI 0487 §D13.2.154.
    let ttbr1: u64;
    unsafe { core::arch::asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nostack, nomem)) };

    // Static call counter — incremented each time we generate entropy.
    // Ensures two rapid calls produce different values even if CNTPCT hasn't
    // advanced (possible in QEMU with fast emulation).
    static CALL_COUNTER: core::sync::atomic::AtomicU64 =
        core::sync::atomic::AtomicU64::new(1);
    let counter = CALL_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    // Mix sources together.
    let mut state = cntpct
        ^ cntfrq.wrapping_mul(0x9e37_79b9_7f4a_7c15)  // Fibonacci hash constant
        ^ (ttbr1 >> 12)                                 // strip low flag bits
        ^ counter.wrapping_mul(0x6c62_272e_07bb_0142);  // prime mixer

    // Xorshift64 — a single pass mixes all bits.
    // Reference: Marsaglia, G. "Xorshift RNGs" (2003).
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;

    state
}

/// Compute an ASLR page offset in the range [0, max_offset_pages).
///
/// Applied to the load base of the ELF image and to the stack base.
/// Reference: Linux arch/arm64/mm/mmap.c arch_mmap_rnd().
fn aslr_page_offset(page_size: u64, max_offset_pages: u64) -> u64 {
    let random_value = read_aslr_entropy();
    let page_offset = random_value % max_offset_pages;
    page_offset * page_size
}

// ---------------------------------------------------------------------------
// LoadedImage — result of a successful load
// ---------------------------------------------------------------------------

pub struct LoadedImage {
    /// ELF entry point virtual address (after ASLR adjustment).
    pub entry_point: u64,
    /// Initial user-space stack pointer.
    ///
    /// Points to the `argc` field of the AArch64 SYSV ABI initial stack layout.
    /// The kernel sets x0 = argc and x1 = argv_va in the exception frame so
    /// that `_start(argc, argv)` receives them as function parameters.
    ///
    ///   [sp + 0]          argc (u64)
    ///   [sp + 8]          argv[0] VA
    ///   [sp + 8*(argc+1)] NULL  (end of argv)
    ///   [sp + 8*(argc+2)] NULL  (end of envp)
    ///   [sp + 8*(argc+3)] string data: "arg0\0arg1\0..."
    pub initial_stack_pointer: u64,
    /// Number of arguments (argc).
    pub argc: usize,
    /// User virtual address of the argv[] pointer array (SP + 8).
    /// Set in x1 by the kernel before jumping to `_start`.
    pub argv_va: u64,
    /// User virtual address of the envp[] pointer array.
    /// Set in x2 by the kernel before jumping to `_start`.
    /// Points to the first `KEY=VALUE\0` string pointer (or the NULL terminator
    /// if the environment is empty).
    pub envp_va: u64,
    /// The newly constructed user page table.
    ///
    /// Returned as a plain value (not Box) so that the caller can box it
    /// outside of the `with_physical_allocator` closure, avoiding re-entrant
    /// access to the global memory state.
    pub page_table: PageTable,
    /// Base virtual address of the demand-paged stack region.
    ///
    /// The physical page at `stack_top - page_size` is mapped immediately (it
    /// holds argv/envp).  The rest of `[stack_demand_base, stack_top - page_size)`
    /// is registered as a demand region: the page fault handler maps pages there
    /// on first access.
    pub stack_demand_base: u64,
    /// Virtual address one past the end of the stack demand region (== stack top).
    pub stack_demand_top: u64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of pages in the user stack.
///
/// 256 pages = 1 MiB.  Increased from 8 (32 KiB) because debug-mode Rust
/// generates large stack frames for Vec/String operations (no inlining).
/// Linux default is 8 MiB; matching that for debug-mode Rust binaries.
/// TECHNICAL DEBT (Fase 5): grow the stack on demand via guard page fault.
const USER_STACK_PAGES: usize = 2048;

/// ASLR range: the load base can be randomly offset by 0–65535 pages (256 MiB).
///
/// Phase 5e: expanded from 256 pages (1 MiB) to 65536 pages (256 MiB).
/// 16 bits of entropy (log₂(65536) = 16) is the minimum Linux uses for
/// non-PIE ASLR on 64-bit platforms (arch/arm64/mm/mmap.c ASLR_BITS).
///
/// Reference: Linux arch/arm64/Kconfig: ARM64_VA_BITS and ELF_ET_DYN_BASE.
const ASLR_MAX_OFFSET_PAGES: u64 = 65536;

/// Base virtual address where user ELF images are loaded.
///
/// The C kernel used 0x400000 (matching Linux defaults for statically linked
/// ELFs).  We keep the same value for compatibility with the existing test
/// binaries.
const USER_IMAGE_BASE: u64 = 0x0000_0000_0040_0000;

/// Virtual address of the user stack top.
///
/// The stack grows downward from this address.  We place it just below the
/// 48-bit user limit (2^47) to leave room for the signal trampoline and mmap.
const USER_STACK_TOP_BASE: u64 = 0x0000_7FFF_FFFF_F000;

// ---------------------------------------------------------------------------
// load_elf()
// ---------------------------------------------------------------------------

/// Parse and load an ELF64 binary into a new user address space.
///
/// Steps:
///   1. Validate ELF header.
///   2. Allocate a new user PageTable.
///   3. For each PT_LOAD segment: allocate physical pages, copy data, map.
///   4. Map the signal trampoline.
///   5. Allocate and map the user stack (with a guard page).
///   6. Return entry point (ASLR-adjusted) + initial SP.
///
/// # Safety
/// `elf_data` must be a valid ELF64 binary.
/// `allocator` must be the active physical allocator with IRQs disabled.
pub unsafe fn load_elf(
    elf_data: &[u8],
    allocator: &mut PhysicalAllocator,
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<LoadedImage, LoaderError> {
    // --- 1. Validate ELF header ---
    if elf_data.len() < core::mem::size_of::<Elf64Header>() {
        return Err(LoaderError::Truncated);
    }

    // Read the header using copy to avoid misalignment UB: the embedded ELF
    // bytes may not be 8-byte aligned in .rodata.
    let header: Elf64Header = core::ptr::read_unaligned(elf_data.as_ptr() as *const Elf64Header);

    if header.e_ident[..4] != ELF_MAGIC {
        return Err(LoaderError::NotAnElf);
    }
    if header.e_ident[4] != ELF_CLASS_64
        || header.e_ident[5] != ELF_DATA_LITTLE_ENDIAN
        || header.e_machine != ELF_MACHINE_AARCH64
    {
        return Err(LoaderError::UnsupportedFormat);
    }
    if header.e_type != ELF_TYPE_EXECUTABLE && header.e_type != ELF_TYPE_SHARED {
        return Err(LoaderError::NotExecutable);
    }
    // PIE binaries (ET_DYN) have segment VAddrs relative to 0; we add
    // USER_IMAGE_BASE so the image lands in the same region as ET_EXEC.
    let is_pie = header.e_type == ELF_TYPE_SHARED;
    let load_base: u64 = if is_pie { USER_IMAGE_BASE } else { 0 };

    // --- 2. Allocate new user page table ---
    let mut page_table = PageTable::new(allocator)
        .map_err(|_| LoaderError::OutOfPhysicalMemory)?;

    let page_size = crate::memory::physical::read_page_size();
    let hhdm_offset = allocator.hhdm_offset();

    // ASLR offset — only applied to PIE binaries (ET_DYN).
    //
    // ET_EXEC binaries have absolute VAs baked into the binary at link time
    // (vtable function pointers, static data pointers in .rodata, etc.).
    // Adding an ASLR offset to a non-PIE binary shifts the code without
    // adjusting those link-time absolute values, causing wrong function-pointer
    // calls at runtime.  PIE binaries (ET_DYN) are position-independent and
    // can safely be loaded at an arbitrary base.
    //
    // Reference: Linux arch/arm64/mm/mmap.c — only ET_DYN binaries get
    // randomised load base (ELF_ET_DYN_BASE).
    let aslr_offset: u64 = if is_pie {
        aslr_page_offset(page_size, ASLR_MAX_OFFSET_PAGES)
    } else {
        0
    };

    // --- 3. Map PT_LOAD segments ---
    let program_header_count = header.e_phnum as usize;
    let program_header_offset = header.e_phoff as usize;
    let program_header_entry_size = header.e_phentsize as usize;

    for index in 0..program_header_count {
        let ph_offset = program_header_offset
            .checked_add(index.checked_mul(program_header_entry_size)
                .ok_or(LoaderError::Truncated)?)
            .ok_or(LoaderError::Truncated)?;
        if ph_offset + core::mem::size_of::<Elf64ProgramHeader>() > elf_data.len() {
            return Err(LoaderError::Truncated);
        }
        let ph: Elf64ProgramHeader = core::ptr::read_unaligned(
            elf_data.as_ptr().add(ph_offset) as *const Elf64ProgramHeader
        );

        if ph.p_type != PT_LOAD {
            continue;
        }

        let segment_vaddr = ph.p_vaddr + load_base + aslr_offset;
        let segment_vaddr_page = align_down(segment_vaddr, page_size);
        let segment_end = segment_vaddr + ph.p_memsz;
        let segment_end_page = align_up(segment_end, page_size);
        let segment_page_count = ((segment_end_page - segment_vaddr_page) / page_size) as usize;

        if segment_vaddr % page_size != ph.p_vaddr % page_size {
            // Page offset within the segment must be preserved.
            // Both must agree on intra-page offset.
            // Actually: ELF spec requires p_vaddr % p_align == p_offset % p_align.
            // We check that p_vaddr is page-aligned for simplicity.
            if ph.p_vaddr % page_size != 0 {
                return Err(LoaderError::UnalignedSegment);
            }
        }

        // Select page flags based on segment permissions.
        let flags = if ph.p_flags & PF_X != 0 {
            PAGE_FLAGS_USER_CODE
        } else {
            PAGE_FLAGS_USER_DATA
        };

        // Allocate physical pages, copy data, map.
        for page_index in 0..segment_page_count {
            let phys = allocator.alloc().ok_or(LoaderError::OutOfPhysicalMemory)?;
            let phys_virt = phys.to_virtual(hhdm_offset).as_ptr::<u8>();

            // Zero the physical page.
            core::ptr::write_bytes(phys_virt, 0, page_size as usize);

            // Copy file data into the page (only up to p_filesz).
            let page_vaddr = segment_vaddr_page + page_index as u64 * page_size;
            // Offset of this page within the segment (in file).
            let segment_page_start = page_vaddr.saturating_sub(segment_vaddr);
            let file_offset = ph.p_offset as usize + segment_page_start as usize;
            let file_bytes_remaining =
                (ph.p_filesz as usize).saturating_sub(segment_page_start as usize);
            let copy_length = file_bytes_remaining.min(page_size as usize);

            if copy_length > 0 && file_offset + copy_length <= elf_data.len() {
                core::ptr::copy_nonoverlapping(
                    elf_data.as_ptr().add(file_offset),
                    phys_virt,
                    copy_length,
                );
            }

            page_table
                .map(
                    VirtualAddress::new(page_vaddr),
                    phys,
                    flags,
                    allocator,
                )
                .map_err(LoaderError::MappingFailed)?;
        }
    }

    // --- 4. Map signal trampoline ---
    // The trampoline is one page at SIGNAL_TRAMPOLINE_VA.
    // It contains one instruction: `svc #SYSCALL_SIGRETURN`.
    {
        let phys = allocator.alloc().ok_or(LoaderError::OutOfPhysicalMemory)?;
        let phys_virt = phys.to_virtual(hhdm_offset).as_ptr::<u32>();
        // Zero the page.
        core::ptr::write_bytes(phys_virt as *mut u8, 0, page_size as usize);
        // Write the trampoline instruction.
        core::ptr::write_volatile(phys_virt, SIGNAL_TRAMPOLINE_INSTRUCTION);
        page_table
            .map(
                VirtualAddress::new(SIGNAL_TRAMPOLINE_VA),
                phys,
                PAGE_FLAGS_USER_CODE,
                allocator,
            )
            .map_err(LoaderError::MappingFailed)?;
    }

    // --- 5. User stack ---
    // Stack top base + random ASLR offset (separate from load ASLR).
    let stack_aslr = aslr_page_offset(page_size, ASLR_MAX_OFFSET_PAGES);
    let stack_top = USER_STACK_TOP_BASE - stack_aslr;
    let stack_bottom = stack_top - USER_STACK_PAGES as u64 * page_size;

    // Demand-paging stack: only the topmost page (which holds argv/envp) is
    // mapped immediately.  All other pages in [stack_bottom, stack_top - page)
    // are left unmapped; the page fault handler will allocate them on first
    // access.  The guard page below stack_bottom is left unmapped as a sentinel.
    //
    // This reduces memory usage from USER_STACK_PAGES * 4 KiB to 1 page at
    // exec time while still providing the full stack virtual address range.
    let top_page_va = stack_top - page_size;
    let stack_top_page_phys = allocator.alloc().ok_or(LoaderError::OutOfPhysicalMemory)?;
    let phys_virt = stack_top_page_phys.to_virtual(hhdm_offset).as_ptr::<u8>();
    core::ptr::write_bytes(phys_virt, 0, page_size as usize);
    page_table
        .map(
            VirtualAddress::new(top_page_va),
            stack_top_page_phys,
            PAGE_FLAGS_USER_DATA,
            allocator,
        )
        .map_err(LoaderError::MappingFailed)?;

    // Build the AArch64 SYSV ABI argv/envp layout in the topmost stack page.
    let (initial_stack_pointer, argc, argv_va, envp_va) = build_argv_on_stack(
        argv,
        envp,
        stack_top_page_phys,
        stack_top,
        hhdm_offset,
        page_size,
    );

    let entry_point = header.e_entry + load_base + aslr_offset;

    // Map the vDSO code page read-only at VDSO_BASE_VA = 0x1000.
    // Userspace branches into this page for all syscalls instead of encoding
    // SVC immediates in compiled binaries (ABI stability).
    // A mapping failure here is non-fatal: userspace falls back to direct SVC.
    let _ = crate::vdso::vdso_map_into_process(&mut page_table, allocator);
    // Map the vDSO data page read-only at 0x3000.
    // Contains boot_rtc_seconds for the fast clock_gettime userspace implementation.
    let _ = crate::vdso::vdso_map_data_into_process(&mut page_table, allocator);

    Ok(LoadedImage {
        entry_point,
        initial_stack_pointer,
        argc,
        argv_va,
        envp_va,
        page_table,
        stack_demand_base: stack_bottom,
        stack_demand_top:  stack_top,
    })
}

// ---------------------------------------------------------------------------
// spawn_from_ramfs — create a user process from a static ELF slice
// ---------------------------------------------------------------------------

/// Create a ready-to-run user process from a static ELF byte slice.
///
/// This is the single entry point for spawning the first user process at boot.
///
/// # Safety
/// Must be called with IRQs disabled and the scheduler initialised.
pub unsafe fn spawn_from_ramfs(
    scheduler: &mut crate::scheduler::Scheduler,
    elf_bytes: &'static [u8],
) -> Result<crate::process::Pid, LoaderError> {
    // 1. Create a new process slot.
    let parent_pid = scheduler.current_pid();
    let child_pid = scheduler.create_process(Some(parent_pid))
        .ok_or(LoaderError::OutOfPhysicalMemory)?;

    // 2. Load the ELF into a new address space.
    // NOTE: `with_physical_allocator` must NOT be called while holding a heap
    // allocation (Box) because GlobalAlloc also accesses the physical allocator
    // internally — aliased mutable references would corrupt allocator state.
    // We return PageTable by value and box it after the closure returns.
    let loaded = crate::memory::with_physical_allocator(|phys| {
        load_elf(elf_bytes, phys, &[], &[])
    })?;
    // Box the page table here, outside with_physical_allocator.
    let page_table_box = Box::new(loaded.page_table);

    // 3. Install the page table and set up the initial frame.
    let child = scheduler.process_mut(child_pid)
        .ok_or(LoaderError::OutOfPhysicalMemory)?;

    child.page_table = Some(page_table_box);

    // Register the demand-paged stack region.  The top page is already mapped
    // (it holds argv/envp); the rest of the range will be faulted in on access.
    let stack_region = crate::process::MmapRegion {
        base:   loaded.stack_demand_base,
        length: loaded.stack_demand_top - loaded.stack_demand_base,
        demand: true,
        shared: false,
        backing: crate::process::MmapBacking::Anonymous,
    };
    child.mmap_regions.insert(loaded.stack_demand_base, stack_region);

    // Build the initial ExceptionFrame on the child's kernel stack.
    use crate::arch::arm64::exceptions::ExceptionFrame;
    let frame_ptr = (child.kernel_stack.top as usize
        - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
    core::ptr::write_bytes(frame_ptr as *mut u8, 0, core::mem::size_of::<ExceptionFrame>());
    (*frame_ptr).elr  = loaded.entry_point;
    (*frame_ptr).spsr = 0;   // EL0 state
    // AArch64 SYSV ABI: x0 = argc, x1 = argv VA (pointer to argv[] array).
    // _start receives these as C function parameters.
    (*frame_ptr).x[0] = loaded.argc as u64;
    (*frame_ptr).x[1] = loaded.argv_va;
    (*frame_ptr).sp = loaded.initial_stack_pointer;

    child.cpu_context.stack_pointer = frame_ptr as u64;
    child.cpu_context.link_register =
        crate::process::process_entry_trampoline_el0 as *const () as u64;
    child.is_foreground = true;

    // 4. Mark the process as ready.
    scheduler.make_ready(child_pid);
    Ok(child_pid)
}

// ---------------------------------------------------------------------------
// spawn_from_vfs — create a user process by reading an ELF from the VFS
// ---------------------------------------------------------------------------

/// Create a ready-to-run user process from a file in the VFS (typically the
/// FAT32 root disk).  The file at `path` is read into a temporary heap buffer
/// and then handed to `load_elf`.
///
/// # Safety
/// Must be called with IRQs disabled and the scheduler initialised.
pub unsafe fn spawn_from_vfs(
    scheduler: &mut crate::scheduler::Scheduler,
    path: &str,
) -> Result<crate::process::Pid, LoaderError> {
    // Resolve the path through the VFS.
    let inode = crate::fs::vfs_resolve(path, None)
        .map_err(|_| LoaderError::NotFound)?;

    // Read the entire ELF into a heap buffer.
    let size = inode.stat().size as usize;
    if size == 0 {
        return Err(LoaderError::NotFound);
    }
    let mut elf_buf = alloc::vec![0u8; size];
    let _ = inode.read_at(0, &mut elf_buf);

    // Load the ELF into a fresh address space.
    let parent_pid = scheduler.current_pid();
    let child_pid = scheduler
        .create_process(Some(parent_pid))
        .ok_or(LoaderError::OutOfPhysicalMemory)?;

    let loaded = crate::memory::with_physical_allocator(|phys| {
        load_elf(&elf_buf, phys, &[], &[])
    })?;
    let page_table_box = Box::new(loaded.page_table);

    // Install page table and initial exception frame (same as spawn_from_ramfs).
    let child = scheduler
        .process_mut(child_pid)
        .ok_or(LoaderError::OutOfPhysicalMemory)?;

    child.page_table = Some(page_table_box);

    // Register the demand-paged stack region.
    let stack_region = crate::process::MmapRegion {
        base:   loaded.stack_demand_base,
        length: loaded.stack_demand_top - loaded.stack_demand_base,
        demand: true,
        shared: false,
        backing: crate::process::MmapBacking::Anonymous,
    };
    child.mmap_regions.insert(loaded.stack_demand_base, stack_region);

    use crate::arch::arm64::exceptions::ExceptionFrame;
    let frame_ptr = (child.kernel_stack.top as usize
        - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
    core::ptr::write_bytes(frame_ptr as *mut u8, 0, core::mem::size_of::<ExceptionFrame>());
    (*frame_ptr).elr  = loaded.entry_point;
    (*frame_ptr).spsr = 0;
    (*frame_ptr).x[0] = loaded.argc as u64;
    (*frame_ptr).x[1] = loaded.argv_va;
    (*frame_ptr).sp   = loaded.initial_stack_pointer;

    child.cpu_context.stack_pointer = frame_ptr as u64;
    child.cpu_context.link_register =
        crate::process::process_entry_trampoline_el0 as *const () as u64;
    child.is_foreground = true;

    scheduler.make_ready(child_pid);
    Ok(child_pid)
}

// ---------------------------------------------------------------------------
// build_argv_on_stack — write AArch64 SYSV initial stack layout
// ---------------------------------------------------------------------------

/// Write argc/argv/envp onto the topmost stack page.
///
/// Returns `(new_sp, argc, argv_va, envp_va)`.
///
/// AArch64 SYSV ABI initial stack layout (all values 8-byte aligned):
///
/// ```
/// [sp + 0]                    argc  (u64)
/// [sp + 8 .. sp+8*(argc+1)]   argv[0..argc-1] VAs
/// [sp + 8*(argc+1)]           NULL  (end of argv[])
/// [sp + 8*(argc+2) ..]        envp[0..envc-1] VAs
/// [sp + 8*(argc+2+envc)]      NULL  (end of envp[])
/// [string area]               "arg0\0arg1\0...\0KEY=VAL\0..."
/// ```
///
/// The entire layout must fit within one page.  Excess entries are silently
/// dropped (argv + envp entries capped at 64 each; total string area 3072 bytes).
///
/// # Safety
/// `stack_top_page_phys` must be the physical address of the topmost mapped
/// stack page.  `stack_top` is the first unmapped virtual address above the
/// stack (i.e. the top of the mapped region is `stack_top - page_size`).
unsafe fn build_argv_on_stack(
    argv: &[&[u8]],
    envp: &[&[u8]],
    stack_top_page_phys: PhysicalAddress,
    stack_top: u64,
    hhdm_offset: u64,
    page_size: u64,
) -> (u64, usize, u64, u64) {
    const MAX_ARGS: usize = 64;
    const MAX_ENV:  usize = 64;
    let argc = argv.len().min(MAX_ARGS);
    let envc = envp.len().min(MAX_ENV);

    // Virtual address of the start of the topmost stack page.
    let page_va = stack_top - page_size;
    // Kernel virtual address of the page (via HHDM) for writing.
    let page_ptr = stack_top_page_phys.to_virtual(hhdm_offset).as_ptr::<u8>();

    // Header layout (pointer arrays):
    //   8 bytes  argc
    //   argc * 8 bytes  argv[] pointers
    //   8 bytes  NULL  (end of argv)
    //   envc * 8 bytes  envp[] pointers
    //   8 bytes  NULL  (end of envp)
    let header_size = 8 * (1 + argc + 1 + envc + 1);
    let string_area_offset = header_size;

    // Write argc.
    let argc_ptr = page_ptr as *mut u64;
    core::ptr::write(argc_ptr, argc as u64);

    // Helper: write one slice array (argv or envp) into the string area.
    // `slot_base` is the byte offset within the page where the first pointer slot lives.
    // Returns the updated string_cursor.
    let write_string_array =
        |strings: &[&[u8]], slot_base: usize, mut cursor: usize| -> usize {
            for (index, s) in strings.iter().enumerate() {
                let remaining = (page_size as usize).saturating_sub(cursor);
                // Empty strings still need a NUL byte in the string area so that
                // the pointer in the slot is non-NULL and points to "\0".
                // An empty string with remaining == 0 is dropped (no space left).
                if remaining == 0 {
                    let slot = page_ptr.add(slot_base + index * 8) as *mut u64;
                    core::ptr::write(slot, 0u64);
                    continue;
                }
                let copy_len = s.len().min(remaining.saturating_sub(1));
                // Write string bytes (may be zero for an empty string).
                if copy_len > 0 {
                    core::ptr::copy_nonoverlapping(s.as_ptr(), page_ptr.add(cursor), copy_len);
                }
                // Always write the NUL terminator.
                *page_ptr.add(cursor + copy_len) = 0u8;
                let string_va = page_va + cursor as u64;
                let slot = page_ptr.add(slot_base + index * 8) as *mut u64;
                core::ptr::write(slot, string_va);
                cursor += copy_len + 1;
            }
            cursor
        };

    // Write argv[] strings and pointer slots.
    let argv_slot_base = 8; // offset 8 within page (after argc)
    let string_cursor = write_string_array(argv, argv_slot_base, string_area_offset);

    // NULL terminator after argv[].
    let argv_null_offset = 8 + argc * 8;
    core::ptr::write(page_ptr.add(argv_null_offset) as *mut u64, 0u64);

    // envp[] starts immediately after the argv NULL.
    let envp_slot_base = argv_null_offset + 8;
    // VA of envp[0] pointer slot (what x2 should hold on entry).
    let envp_va = page_va + envp_slot_base as u64;

    let _string_cursor = write_string_array(envp, envp_slot_base, string_cursor);

    // NULL terminator after envp[].
    let envp_null_offset = envp_slot_base + envc * 8;
    core::ptr::write(page_ptr.add(envp_null_offset) as *mut u64, 0u64);

    let new_sp  = page_va;
    let argv_va = page_va + 8;

    (new_sp, argc, argv_va, envp_va)
}

// ---------------------------------------------------------------------------
// Alignment helpers
// ---------------------------------------------------------------------------

#[inline]
fn align_down(value: u64, alignment: u64) -> u64 {
    value & !(alignment - 1)
}

#[inline]
fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}
