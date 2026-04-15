// memory.rs — Memory management syscall implementations.
//
// Syscalls: mmap, munmap

use super::*;

// ---------------------------------------------------------------------------
// sys_mmap — memory mapping (anonymous and file-backed)
//
// ABI: mmap(addr, length, prot, flags, fd, offset)
//   addr:   hint (ignored; we use a bump pointer).
//   length: bytes to map (rounded up to a page boundary).
//   prot:   PROT_READ / PROT_WRITE / PROT_EXEC (stored but not enforced yet).
//   flags:  MAP_ANONYMOUS, MAP_PRIVATE, MAP_SHARED, MAP_FIXED (MAP_FIXED ignored).
//   fd:     file descriptor for file-backed mappings; -1 for anonymous.
//   offset: byte offset into the file (must be page-aligned for file-backed).
//
// Supported combinations:
//   MAP_ANONYMOUS | MAP_PRIVATE  — zero-filled anonymous (demand).
//   MAP_ANONYMOUS | MAP_SHARED   — anonymous shared (registered in SharedRegionTable).
//   fd >= 0 | MAP_PRIVATE        — file-backed CoW (demand, reads from inode on fault).
//   fd >= 0 | MAP_SHARED         — file-backed shared (writes flush to inode on msync/munmap).
//
// Reference: POSIX.1-2017 mmap(2).
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_mmap(
    _addr: u64,
    length: u64,
    _prot: i32,
    flags: i32,
    fd: i32,
    offset: u64,
) -> i64 {
    if length == 0 {
        return EINVAL;
    }

    let page_size = crate::memory::physical::read_page_size();
    let pages = ((length + page_size - 1) / page_size) as usize;

    let is_anonymous = (flags & MAP_ANONYMOUS != 0) || fd < 0;
    let is_shared = flags & MAP_SHARED != 0;
    let is_shared_anonymous = is_shared && is_anonymous;

    // --- File-backed MAP_PRIVATE ---
    // Resolve the inode now (outside the scheduler lock) so we can store an
    // Arc<dyn Inode> in the MmapRegion backing.
    let file_backing: Option<alloc::sync::Arc<dyn crate::fs::Inode>> = if !is_anonymous {
        // Validate fd and offset alignment.
        if offset % page_size != 0 {
            return EINVAL;
        }
        let inode = crate::scheduler::with_scheduler(|scheduler| {
            scheduler.current_process().and_then(|p| {
                let table = p.file_descriptor_table.lock();
                table.get(fd as usize).and_then(|desc| {
                    match desc {
                        crate::fs::vfs::FileDescriptor::InoFile { inode, .. } => {
                            Some(alloc::sync::Arc::clone(inode))
                        }
                        _ => None,
                    }
                })
            })
        });
        match inode {
            Some(inode) => Some(inode),
            None => return EBADF,
        }
    } else {
        None
    };

    // Allocate VA space via bump pointer; all regions are demand-paged.
    let base_va_opt = crate::scheduler::with_scheduler(|scheduler| {
        let process = scheduler.current_process_mut()?;
        if process.mmap_regions.len() >= crate::process::MMAP_MAX_REGIONS {
            return None;
        }
        let base = process.mmap_next_va;
        let region_length = pages as u64 * page_size;
        process.mmap_next_va = base + region_length;

        let backing = if let Some(ref inode) = file_backing {
            crate::process::MmapBacking::File {
                inode: alloc::sync::Arc::clone(inode),
                file_offset: offset,
            }
        } else {
            crate::process::MmapBacking::Anonymous
        };

        let region = crate::process::MmapRegion {
            base,
            length: region_length,
            demand: true,
            shared: is_shared,
            backing,
        };
        process.mmap_regions.insert(base, region);

        Some(base)
    });

    let base_va = match base_va_opt {
        Some(va) => va,
        None => return ENOMEM,
    };

    if is_shared_anonymous {
        // Register the region so fork() will map it shared rather than CoW.
        let table = &mut *SHARED_REGION_TABLE.0.get();
        table.insert(base_va, SharedRegion {
            phys_base: 0, // tracked by page table; placeholder
            page_count: pages,
            reference_count: 1,
        });
    }

    base_va as i64
}

// ---------------------------------------------------------------------------
// sys_munmap
// ---------------------------------------------------------------------------

pub(super) unsafe fn sys_munmap(addr: u64, length: u64) -> i64 {
    if addr == 0 || length == 0 {
        return EINVAL;
    }

    let page_size = crate::memory::physical::read_page_size();

    // POSIX.1-2017 munmap(2): "The addr argument shall be a multiple of the
    // page size as returned by sysconf(_SC_PAGESIZE)."
    // Linux returns EINVAL for a non-page-aligned addr.
    if addr % page_size != 0 {
        return EINVAL;
    }
    let pages = ((length + page_size - 1) / page_size) as usize;

    crate::memory::with_physical_allocator(|phys| {
        crate::scheduler::with_scheduler(|scheduler| {
            if scheduler.munmap_for_current(addr, pages, page_size, phys) {
                0
            } else {
                EINVAL
            }
        })
    })
}

// ---------------------------------------------------------------------------
// sys_fork
// ---------------------------------------------------------------------------
