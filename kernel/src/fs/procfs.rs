// fs/procfs.rs — Process information virtual filesystem.
//
// Paths supported:
//   <pid>/status          — process state snapshot.
//   <pid>/maps            — virtual memory map for the process.
//   meminfo               — system memory statistics.
//   cpuinfo               — CPU description from MIDR_EL1.
//   uptime                — seconds since boot.

extern crate alloc;

use alloc::vec::Vec;
use alloc::format;
use alloc::string::String;

pub struct ProcSnapshot {
    pub data: Vec<u8>,
    pub position: usize,
}

/// Open a procfs path.
///
/// Supported forms:
///   "//proc:<pid>/status"   → process status text
///   "//proc:<pid>/maps"     → virtual memory regions
///   "//proc:meminfo"        → memory statistics
///   "//proc:cpuinfo"        → CPU information
///   "//proc:uptime"         → seconds since boot
///
/// Also accepts the path without the "//proc:" prefix (bare e.g. "1/status").
///
/// Returns a ProcSnapshot or None if not found.
pub fn procfs_open(path: &str) -> Option<ProcSnapshot> {
    // Strip leading "//proc:" or just parse directly.
    let path = if let Some(rest) = path.strip_prefix("//proc:") {
        rest
    } else {
        path
    };

    // --- Flat (non-per-process) entries ---
    match path {
        "meminfo" => return procfs_meminfo(),
        "cpuinfo" => return procfs_cpuinfo(),
        "uptime"  => return procfs_uptime(),
        _ => {}
    }

    // --- Per-process entries: "<pid>/<entry>" ---
    let slash_position = path.find('/')?;
    let pid_str = &path[..slash_position];
    let rest    = &path[slash_position + 1..];

    let pid_index: u16 = pid_str.parse().ok()?;
    if pid_index == 0 {
        return None;
    }

    match rest {
        "status" => procfs_process_status(pid_index),
        "maps"   => procfs_process_maps(pid_index),
        _        => None,
    }
}

// ---------------------------------------------------------------------------
// /proc/<pid>/status
// ---------------------------------------------------------------------------

fn procfs_process_status(pid_index: u16) -> Option<ProcSnapshot> {
    let content = unsafe { crate::scheduler::with_scheduler(|scheduler| {
        let pid = crate::process::Pid::new(pid_index, 1);
        let process = scheduler.process(pid)?;

        let state_name = match process.state {
            crate::process::ProcessState::Ready           => "ready",
            crate::process::ProcessState::Running         => "running",
            crate::process::ProcessState::Blocked         => "blocked",
            crate::process::ProcessState::Waiting { .. }  => "waiting",
            crate::process::ProcessState::Zombie { .. }   => "zombie",
            crate::process::ProcessState::Sleeping { .. } => "sleeping",
            crate::process::ProcessState::Stopped         => "stopped",
        };

        let parent_index = process.parent_pid
            .map(|p| p.index as u64)
            .unwrap_or(0);

        let text = format!(
            "pid: {}\nppid: {}\nstate: {}\n",
            process.pid.index,
            parent_index,
            state_name,
        );
        Some(text.into_bytes())
    }) };

    let data = content?;
    Some(ProcSnapshot { data, position: 0 })
}

// ---------------------------------------------------------------------------
// /proc/<pid>/maps
// ---------------------------------------------------------------------------

fn procfs_process_maps(pid_index: u16) -> Option<ProcSnapshot> {
    let content = unsafe { crate::scheduler::with_scheduler(|scheduler| {
        let pid = crate::process::Pid::new(pid_index, 1);
        let process = scheduler.process(pid)?;

        let mut text = String::new();
        for region in process.mmap_regions.values() {
            let start = region.base;
            let end   = region.base.saturating_add(region.length);
            // Anonymous mappings: report as rw-p (readable, writable, not
            // executable, private).  We do not track per-region permission
            // bits in the current design.
            text.push_str(&format!(
                "{:016x}-{:016x} rw-p 00000000 00:00 0\n",
                start,
                end,
            ));
        }
        Some(text.into_bytes())
    }) };

    let data = content?;
    Some(ProcSnapshot { data, position: 0 })
}

// ---------------------------------------------------------------------------
// /proc/meminfo
// ---------------------------------------------------------------------------

fn procfs_meminfo() -> Option<ProcSnapshot> {
    let (total_bytes, free_bytes) = crate::memory::physical_stats();
    let total_kb = total_bytes / 1024;
    let free_kb  = free_bytes  / 1024;

    let text = format!(
        "MemTotal:     {} kB\nMemFree:      {} kB\nMemAvailable: {} kB\n",
        total_kb,
        free_kb,
        free_kb,
    );
    Some(ProcSnapshot { data: text.into_bytes(), position: 0 })
}

// ---------------------------------------------------------------------------
// /proc/cpuinfo
// ---------------------------------------------------------------------------

fn procfs_cpuinfo() -> Option<ProcSnapshot> {
    // Read MIDR_EL1 to obtain implementer, variant, architecture, part, revision.
    //
    // MIDR_EL1 bit layout (ARM ARM DDI 0487 D13.2.81):
    //   [31:24] Implementer
    //   [23:20] Variant
    //   [19:16] Architecture
    //   [15:4]  PartNum
    //   [3:0]   Revision
    let midr: u64;
    unsafe { core::arch::asm!("mrs {}, midr_el1", out(reg) midr, options(nostack, nomem)) };
    let cpu_implementer  = (midr >> 24) & 0xFF;
    let cpu_variant      = (midr >> 20) & 0xF;
    let cpu_architecture = (midr >> 16) & 0xF;
    let cpu_part         = (midr >> 4)  & 0xFFF;
    let cpu_revision     = midr         & 0xF;

    // Number of logical processors = BSP (1) + APs.
    let cpu_count = 1 + crate::smp::AP_ONLINE_COUNT
        .load(core::sync::atomic::Ordering::Relaxed);

    let mut text = String::new();
    for processor_index in 0..cpu_count {
        text.push_str(&format!(
            "processor\t: {}\n\
             BogoMIPS\t: 100.00\n\
             Features\t: fp asimd evtstrm\n\
             CPU implementer\t: {:#04x}\n\
             CPU architecture: {}\n\
             CPU variant\t: {:#03x}\n\
             CPU part\t: {:#05x}\n\
             CPU revision\t: {}\n\
             \n",
            processor_index,
            cpu_implementer,
            cpu_architecture,
            cpu_variant,
            cpu_part,
            cpu_revision,
        ));
    }
    Some(ProcSnapshot { data: text.into_bytes(), position: 0 })
}

// ---------------------------------------------------------------------------
// /proc/uptime
// ---------------------------------------------------------------------------

fn procfs_uptime() -> Option<ProcSnapshot> {
    // current_tick() returns the number of TICK_INTERVAL_MS (10 ms) intervals
    // elapsed since boot.
    let tick = crate::platform::qemu_virt::timer::current_tick();
    let tick_interval_ms = crate::platform::qemu_virt::timer::TICK_INTERVAL_MS;

    let total_ms = tick.saturating_mul(tick_interval_ms);
    let seconds      = total_ms / 1000;
    let centiseconds = (total_ms % 1000) / 10;

    // Format: "<seconds>.<centiseconds_2digits> <seconds>.<centiseconds_2digits>\n"
    // The second field is idle time; we approximate it as the same value.
    let text = format!(
        "{}.{:02} {}.{:02}\n",
        seconds, centiseconds,
        seconds, centiseconds,
    );
    Some(ProcSnapshot { data: text.into_bytes(), position: 0 })
}

// ---------------------------------------------------------------------------
// ProcSnapshot::read
// ---------------------------------------------------------------------------

impl ProcSnapshot {
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let available = self.data.len().saturating_sub(self.position);
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&self.data[self.position..self.position + to_read]);
        self.position += to_read;
        to_read
    }
}

// ---------------------------------------------------------------------------
// Inode-based procfs for VFS integration
//
// Mounted at "/proc" by vfs_init(). Provides:
//   /proc/self          → SymlinkInode → /proc/<current_pid>
//   /proc/<pid>/        → ProcPidDirInode
//   /proc/<pid>/comm    → process name (null-terminated, newline-stripped)
//   /proc/<pid>/status  → text snapshot
//   /proc/<pid>/maps    → memory map text snapshot
//   /proc/meminfo       → system memory snapshot
//   /proc/cpuinfo       → CPU info snapshot
//   /proc/uptime        → uptime snapshot
// ---------------------------------------------------------------------------

use crate::fs::inode::{
    alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType, SymlinkInode,
};
use alloc::sync::Arc;

// ---------------------------------------------------------------------------
// ProcfsRootInode — the /proc directory
// ---------------------------------------------------------------------------

/// Root inode for the virtual `/proc` filesystem.
///
/// Mounted at "/proc" during `vfs_init()`. All accesses to `/proc/...` paths
/// go through this inode's `lookup()` and `readdir()`.
pub struct ProcfsRootInode {
    inode_number: u64,
}

unsafe impl Send for ProcfsRootInode {}
unsafe impl Sync for ProcfsRootInode {}

impl ProcfsRootInode {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number() })
    }
}

impl Inode for ProcfsRootInode {
    fn inode_type(&self) -> InodeType { InodeType::Directory }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: 0,
            mode: 0o040555,  // dr-xr-xr-x
            nlinks: 2,
            uid: 0,
            gid: 0,
        }
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        if name == "self" {
            // Dynamic symlink: target resolves to /proc/<current_pid>.
            let pid = unsafe { crate::scheduler::with_scheduler(|s| s.current_pid().index) };
            let target = alloc::format!("/proc/{}", pid);
            return Some(SymlinkInode::new(target));
        }

        // Static flat entries.
        let snapshot = match name {
            "meminfo" => procfs_meminfo(),
            "cpuinfo" => procfs_cpuinfo(),
            "uptime"  => procfs_uptime(),
            _ => None,
        };
        if let Some(snap) = snapshot {
            return Some(ProcSnapshotInode::new(snap.data));
        }

        // Numeric PID directory.
        if let Ok(pid_index) = name.parse::<u16>() {
            if pid_index > 0 {
                return Some(ProcPidDirInode::new(pid_index));
            }
        }
        None
    }

    fn readdir(&self, offset: usize) -> Option<DirEntry> {
        if offset == 0 {
            return Some(DirEntry {
                name: alloc::string::String::from("self"),
                inode_type: InodeType::Symlink,
                inode_number: 0,
            });
        }

        // Enumerate running PIDs.
        let pids = unsafe { crate::scheduler::with_scheduler(|s| s.list_pids()) };
        let index = offset - 1;
        pids.get(index).map(|&pid_index| DirEntry {
            name: alloc::format!("{}", pid_index),
            inode_type: InodeType::Directory,
            inode_number: 0,
        })
    }

    fn read_at(&self, _o: u64, _b: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn write_at(&self, _o: u64, _b: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn create(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn mkdir(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn unlink(&self, _n: &str) -> Result<(), FsError> { Err(FsError::NotSupported) }
    fn truncate(&self, _s: u64) -> Result<(), FsError> { Err(FsError::NotSupported) }
}

// ---------------------------------------------------------------------------
// ProcPidDirInode — /proc/<pid>/
// ---------------------------------------------------------------------------

struct ProcPidDirInode {
    inode_number: u64,
    pid_index: u16,
}

unsafe impl Send for ProcPidDirInode {}
unsafe impl Sync for ProcPidDirInode {}

impl ProcPidDirInode {
    fn new(pid_index: u16) -> Arc<Self> {
        Arc::new(Self { inode_number: alloc_inode_number(), pid_index })
    }
}

impl Inode for ProcPidDirInode {
    fn inode_type(&self) -> InodeType { InodeType::Directory }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: 0,
            mode: 0o040555,
            nlinks: 2,
            uid: 0,
            gid: 0,
        }
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let pid = self.pid_index;
        match name {
            "comm" => {
                let data = unsafe { crate::scheduler::with_scheduler(|s| {
                    s.pool_get_by_index(pid as usize).map(|p| {
                        let name_bytes = &p.name;
                        let len = name_bytes.iter().position(|&b| b == 0)
                            .unwrap_or(name_bytes.len());
                        let mut v = alloc::vec::Vec::from(&name_bytes[..len]);
                        v.push(b'\n');
                        v
                    })
                }) }.unwrap_or_else(|| b"?\n".to_vec());
                Some(ProcSnapshotInode::new(data))
            }
            "status" => {
                procfs_process_status(pid)
                    .map(|snap| ProcSnapshotInode::new(snap.data) as Arc<dyn Inode>)
            }
            "maps" => {
                procfs_process_maps(pid)
                    .map(|snap| ProcSnapshotInode::new(snap.data) as Arc<dyn Inode>)
            }
            _ => None,
        }
    }

    fn readdir(&self, offset: usize) -> Option<DirEntry> {
        let entries = [("comm", InodeType::RegularFile),
                       ("status", InodeType::RegularFile),
                       ("maps", InodeType::RegularFile)];
        entries.get(offset).map(|(name, t)| DirEntry {
            name: alloc::string::String::from(*name),
            inode_type: *t,
            inode_number: 0,
        })
    }

    fn read_at(&self, _o: u64, _b: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn write_at(&self, _o: u64, _b: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn create(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn mkdir(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn unlink(&self, _n: &str) -> Result<(), FsError> { Err(FsError::NotSupported) }
    fn truncate(&self, _s: u64) -> Result<(), FsError> { Err(FsError::NotSupported) }
}

// ---------------------------------------------------------------------------
// ProcSnapshotInode — read-only in-memory file backed by snapshot bytes
// ---------------------------------------------------------------------------

struct ProcSnapshotInode {
    inode_number: u64,
    data: core::cell::UnsafeCell<alloc::vec::Vec<u8>>,
}

unsafe impl Send for ProcSnapshotInode {}
unsafe impl Sync for ProcSnapshotInode {}

impl ProcSnapshotInode {
    fn new(data: alloc::vec::Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            data: core::cell::UnsafeCell::new(data),
        })
    }
}

impl Inode for ProcSnapshotInode {
    fn inode_type(&self) -> InodeType { InodeType::RegularFile }

    fn stat(&self) -> InodeStat {
        let len = unsafe { (*self.data.get()).len() as u64 };
        InodeStat {
            inode_number: self.inode_number,
            size: len,
            mode: 0o100444,  // -r--r--r--
            nlinks: 1,
            uid: 0,
            gid: 0,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let data = unsafe { &*self.data.get() };
        let start = offset as usize;
        if start >= data.len() { return Ok(0); }
        let count = (data.len() - start).min(buf.len());
        buf[..count].copy_from_slice(&data[start..start + count]);
        Ok(count)
    }

    fn write_at(&self, _o: u64, _b: &[u8]) -> Result<usize, FsError> { Err(FsError::NotSupported) }
    fn lookup(&self, _n: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _o: usize) -> Option<DirEntry> { None }
    fn create(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn mkdir(&self, _n: &str) -> Result<Arc<dyn Inode>, FsError> { Err(FsError::NotSupported) }
    fn unlink(&self, _n: &str) -> Result<(), FsError> { Err(FsError::NotSupported) }
    fn truncate(&self, _s: u64) -> Result<(), FsError> { Err(FsError::NotSupported) }
}
