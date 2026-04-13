// process/mod.rs — Process model for Bazzulto OS.
//
// Every schedulable entity is a Process. Kernel-only tasks (idle process)
// have `page_table = None`. User processes have a TTBR0-backed PageTable.
//
// Safety invariants:
//   - `cpu_context` is only valid (and must only be read by the scheduler)
//     when `state != ProcessState::Running`. While running, the CPU holds
//     the actual register state.
//   - `pending_signals` is written with `fetch_or(AcqRel)` from any context,
//     including IRQ handlers. It is read/cleared exclusively in process context.
//   - `signal_handlers` is read and written only from process context, protected
//     by the scheduler lock held around process struct access.
//   - `KernelStack` is accessed exclusively by the process's kernel thread.
//     Never shared.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::memory::virtual_memory::PageTable;
use crate::fs::vfs::FileDescriptorTable;
use crate::sync::SpinLock;

// ---------------------------------------------------------------------------
// Size constants
// ---------------------------------------------------------------------------

/// Kernel stack size per process.
///
/// Reference:
///   Linux aarch64: 16 KiB minimum, 64 KiB with CONFIG_VMAP_STACK.
///   macOS arm64: 64 KiB per thread.
///
/// 64 KiB provides adequate call-chain depth during kernel development.
pub const KERNEL_STACK_SIZE: usize = 64 * 1024;

/// Size of the kernel stack guard page.
///
/// Placed immediately below (lower address than) the kernel stack allocation.
/// Any stack overflow writes into this unmapped page, causing a Data Abort
/// that kills the process rather than silently corrupting heap or other stacks.
///
/// Must match the kernel page size (4 KiB for 4K granule).
/// Reference: Linux arch/arm64/mm/mmu.c, VMAP_STACK guard page.
pub const KERNEL_STACK_GUARD_SIZE: usize = 4096;

/// Maximum number of signals (POSIX 1–31 + Linux real-time 32–64).
///
/// Signal 0 is unused (reserved for kill() "send no signal" semantic).
/// We support 64 signals matching Linux's NSIG.
pub const SIGNAL_COUNT: usize = 64;

/// Upper bound of valid user virtual addresses (exclusive).
///
/// AArch64 with TCR_EL1.T0SZ = 16: TTBR0 covers [0, 2^48).
/// Handlers registered above this limit are rejected to prevent user code
/// from redirecting signal delivery into kernel virtual address space.
///
/// Reference: ARM ARM DDI 0487 D5.2 "Translation regime for EL0".
pub const USER_ADDR_LIMIT: u64 = 0x0001_0000_0000_0000;

/// Base virtual address for anonymous mmap allocations in user space.
///
/// Placed at 8 GiB, well above typical ELF load addresses (400000–800000)
/// and well below the stack (near USER_ADDR_LIMIT).
///
/// TECHNICAL DEBT (Fase 5): Linux derives this dynamically from the binary
/// layout and applies ASLR. We use a fixed base for simplicity in Fase 4.
pub const MMAP_USER_BASE: u64 = 0x0000_0002_0000_0000;

/// Maximum anonymous mmap regions per process.
///
/// With the slab allocator in Bazzulto.System, small allocations (≤ 4096 B)
/// are served from 64 KB arenas — one kernel region per arena.  Ten size
/// classes × a handful of arenas each = well under 100 regions for normal
/// workloads.  Large allocations use one region each; 1024 regions supports
/// thousands of concurrent large objects.
///
/// A proper VMA tree (like Linux `mm_struct`) that merges adjacent same-
/// permission regions would remove the need for this cap entirely.
/// Reference: Linux kernel, mm/mmap.c, `vm_area_struct`.
///
/// TECHNICAL DEBT (Fase 5): Replace flat Vec with a VMA tree.
pub const MMAP_MAX_REGIONS: usize = 1024;

// ---------------------------------------------------------------------------
// Capability constants
// ---------------------------------------------------------------------------

/// Map the boot-time framebuffer into the process's address space.
/// Required by the display server (bzdisplayd).
pub const CAP_DISPLAY: u64 = 1 << 0;

/// Grant capabilities to child processes via sys_spawn.
/// Only bzinit (PID 1) holds this at boot.
pub const CAP_SETCAP:  u64 = 1 << 1;

/// All capabilities combined — granted to bzinit at boot.
pub const CAP_ALL: u64 = CAP_DISPLAY | CAP_SETCAP;

/// Maximum number of open file descriptors per process.
///
/// Reference:
///   Linux default RLIMIT_NOFILE: 1024 soft, 1048576 hard.
///   macOS: 256 soft, 10240 hard.
///   The C kernel used 64 — too small for I/O-heavy programs.
pub const MAX_OPEN_FILE_DESCRIPTORS: usize = 1024;

// ---------------------------------------------------------------------------
// Pid
// ---------------------------------------------------------------------------

/// Process identifier.
///
/// Combines a pool index (u16, up to PID_MAX = 32768) with a generation
/// counter (u8) to prevent the ABA problem: if slot 42 is reused after a
/// process exits, the new occupant has generation = old + 1, making any
/// stale Pid values held by other processes visibly stale to wait() and kill().
///
/// PID_MAX = 32768 matches Linux's default (PID_MAX_DEFAULT).
/// macOS uses ~99999; Windows uses 65536. We start at Linux's default.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pid {
    /// Pool slot index (0–32767).
    pub index: u16,
    /// Generation counter (wraps at 255).
    pub generation: u8,
}

impl Pid {
    /// PID of the idle process.  Always slot 0, generation 0.
    pub const IDLE: Pid = Pid { index: 0, generation: 0 };

    pub const fn new(index: u16, generation: u8) -> Self {
        Self { index, generation }
    }
}

impl core::fmt::Display for Pid {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{}", self.index)
    }
}

// ---------------------------------------------------------------------------
// ProcessState
// ---------------------------------------------------------------------------

/// Lifecycle state of a process.
///
/// State transitions:
///   Ready    ↔ Running          (normal scheduling round-trip)
///   Running  →  Blocked  → Ready (I/O wait → IRQ/event wakeup)
///   Running  →  Waiting  → Ready (waitpid → child exits, signals wake-up)
///   Running  →  Zombie          (exit() called; slot held for parent wait())
///   Zombie   →  (slot freed)    (parent called wait(); slot returned to pool)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    /// Eligible to run; present in the run queue.
    Ready,

    /// Currently executing on the CPU.
    ///
    /// Invariant: exactly one process is in Running state at any time
    /// on a single-core system.
    Running,

    /// Waiting for an I/O event (pipe data, device ready, sleep timeout).
    ///
    /// Not in the run queue. The event source is responsible for
    /// transitioning this process back to Ready when the event fires.
    Blocked,

    /// Blocked in a wait()/waitpid() syscall.
    ///
    /// `for_pid` is None for wait() (any child) or Some(pid) for waitpid().
    Waiting { for_pid: Option<Pid> },

    /// Sleeping until the kernel tick counter reaches `wake_at_tick`.
    ///
    /// Transitioned to Ready by the scheduler when
    /// `current_tick() >= wake_at_tick`.
    Sleeping { wake_at_tick: u64 },

    /// Stopped by SIGSTOP or SIGTSTP.
    ///
    /// The process remains in this state until SIGCONT is delivered.
    /// `waitpid(WUNTRACED)` returns for a newly stopped child.
    Stopped,

    /// Process has called exit(); holding exit code for parent's wait().
    ///
    /// The process slot and all resources except the exit code are freed.
    /// The slot is reclaimed when the parent calls wait() or when the
    /// parent itself exits (children reparented to PID 1).
    Zombie { exit_code: i32 },
}

// ---------------------------------------------------------------------------
// SignalAction
// ---------------------------------------------------------------------------

/// Disposition of a signal for a specific process.
#[derive(Clone, Copy, Debug)]
pub enum SignalAction {
    /// Perform the default OS action for this signal.
    ///
    /// Default actions vary by signal:
    ///   - Most signals: terminate the process.
    ///   - SIGCHLD, SIGURG: ignore.
    ///   - SIGSTOP, SIGTSTP: stop the process (Fase 5).
    ///   - SIGCONT: continue if stopped (Fase 5).
    Default,

    /// Explicitly ignored.  Signal is discarded on delivery.
    Ignore,

    /// Call user-space function at this virtual address.
    ///
    /// Invariant: `va < USER_ADDR_LIMIT`.
    /// Enforced at registration time by `sys_sigaction()`.
    ///
    /// `on_stack` is true when the caller passed `SA_ONSTACK` in `sa_flags`.
    /// When true and `process.signal_stack` is set and the process is not
    /// already executing on the alternate stack, the kernel switches the user
    /// stack pointer to the alternate stack before delivering the signal.
    Handler { va: u64, on_stack: bool },
}

// SAFETY: All variants are plain data (no heap pointers).
unsafe impl Send for SignalAction {}

// ---------------------------------------------------------------------------
// SignalStack — alternate signal delivery stack
// ---------------------------------------------------------------------------

/// Per-process alternate signal stack.
///
/// When a signal is delivered to a handler registered with `SA_ONSTACK` and
/// this struct is populated (flags does not include `SS_DISABLE`), the kernel
/// switches the user stack pointer to `base + size` before calling the handler.
///
/// Reference: POSIX.1-2017 `sigaltstack(2)`.
#[derive(Clone, Copy, Debug)]
pub struct SignalStack {
    /// Base (lowest) virtual address of the alternate stack region.
    pub base: u64,
    /// Size of the alternate stack region in bytes.
    pub size: usize,
    /// Status flags: `SS_DISABLE` (4) = stack is disabled.
    pub flags: u32,
}

/// `ss_flags` value: the alternate stack is disabled (not set).
pub const SS_DISABLE: u32 = 4;
/// `ss_flags` value returned while executing on the alternate stack.
pub const SS_ONSTACK: u32 = 1;

/// `sa_flags` bit: deliver signal on the alternate stack.
///
/// Reference: POSIX.1-2017 `sigaction(2)`.
pub const SA_ONSTACK: u32 = 0x08000000;

impl Default for SignalAction {
    fn default() -> Self {
        SignalAction::Default
    }
}

// ---------------------------------------------------------------------------
// ResourceLimits
// ---------------------------------------------------------------------------

/// Per-process resource limits (subset of POSIX RLIMIT_*).
///
/// Each field is a soft limit in its natural unit.  `u64::MAX` means unlimited.
///
/// Reference: POSIX.1-2017 `getrlimit(2)`, Linux `kernel/sys.c`.
#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    /// RLIMIT_AS — maximum virtual address space in bytes.
    ///
    /// Enforced in `sys_mmap()` when the total mapped size would exceed this.
    /// Linux default: unlimited.  macOS default: unlimited.
    pub address_space_bytes: u64,

    /// RLIMIT_STACK — maximum stack size in bytes.
    ///
    /// Enforced at stack allocation time. Linux default: 8 MiB.
    pub stack_bytes: u64,

    /// RLIMIT_NOFILE — maximum number of open file descriptors.
    ///
    /// Enforced in `sys_open()` / `sys_pipe()`. Linux default: 1024.
    pub open_files: u64,
}

impl ResourceLimits {
    /// Conservative defaults matching Linux's soft limits.
    pub const fn default_limits() -> Self {
        Self {
            address_space_bytes: u64::MAX,
            stack_bytes: 8 * 1024 * 1024, // 8 MiB
            open_files: 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// RLIMIT resource IDs (matching Linux values for ABI compatibility)
// ---------------------------------------------------------------------------

/// `RLIMIT_NOFILE` — maximum open file descriptors.
pub const RLIMIT_NOFILE: u32 = 7;
/// `RLIMIT_AS` — maximum virtual address space.
pub const RLIMIT_AS: u32 = 9;
/// `RLIMIT_STACK` — maximum stack size.
pub const RLIMIT_STACK: u32 = 3;

// ---------------------------------------------------------------------------
// Nice / priority constants
// ---------------------------------------------------------------------------

/// Range of nice values: [NICE_MIN, NICE_MAX].
///
/// Matches Linux: nice ranges from -20 (highest priority) to 19 (lowest).
pub const NICE_MIN: i8 = -20;
pub const NICE_MAX: i8 = 19;

/// Default nice value for new processes.
pub const NICE_DEFAULT: i8 = 0;

/// Number of priority levels in the multilevel run queue.
///
/// Maps nice -20 → level 0 (highest), nice 19 → level 39 (lowest).
/// `priority_level = nice - NICE_MIN` = `nice + 20`.
pub const PRIORITY_LEVELS: usize = 40;

/// Convert a nice value to a run-queue priority level.
///
/// Level 0 = highest priority (nice -20), level 39 = lowest (nice 19).
#[inline]
pub fn nice_to_priority_level(nice: i8) -> usize {
    (nice.wrapping_sub(NICE_MIN)) as usize
}

// ---------------------------------------------------------------------------
// MmapRegion
// ---------------------------------------------------------------------------

/// One anonymous mmap region owned by a process.
///
/// Used to validate `munmap()` addresses and to clean up the address space
/// on process exit.
#[derive(Clone, Copy, Debug)]
pub struct MmapRegion {
    /// Starting virtual address of the region (page-aligned).
    pub base: u64,
    /// Length in bytes (page-aligned).
    pub length: u64,
}

// ---------------------------------------------------------------------------
// CpuContext
// ---------------------------------------------------------------------------

/// Saved CPU register state for kernel-mode context switching.
///
/// Layout matches `context_switch` in `process/context_switch.S` exactly.
/// A compile-time size assertion below verifies that neither drifts.
///
/// Only callee-saved registers are preserved across a context switch,
/// consistent with the AAPCS64 calling convention.
///
/// `link_register` (x30) serves as the "return address" after context_switch:
/// when a process is switched to for the first time, `link_register` must
/// point to the appropriate entry trampoline.
#[repr(C)]
pub struct CpuContext {
    /// x19–x28: callee-saved general-purpose registers.
    /// Layout: gp_regs[0] = x19, gp_regs[1] = x20, …, gp_regs[9] = x28.
    pub gp_regs: [u64; 10],
    /// x29: frame pointer (callee-saved per AAPCS64).
    pub frame_pointer: u64,
    /// x30: link register.
    ///
    /// For a process that has run before: the return address into the
    /// scheduler's `context_switch` call site.
    /// For a new process: the address of the appropriate entry trampoline
    /// (`process_entry_trampoline_el0` for user processes).
    pub link_register: u64,
    /// SP: stack pointer at the moment of context switch.
    ///
    /// For a new user process: points to the ExceptionFrame placed at the
    /// top of the kernel stack by fork()/exec(), which the entry trampoline
    /// will restore and eret from.
    pub stack_pointer: u64,
}

// Compile-time check: CpuContext must be exactly 104 bytes.
// The assembly in context_switch.S uses hard-coded offsets (0, 16, 32, …, 96).
// If a field is added or removed, update both this struct and the assembly.
const _: () = {
    if core::mem::size_of::<CpuContext>() != 104 {
        panic!("CpuContext size mismatch — update context_switch.S offsets");
    }
};

impl CpuContext {
    /// Zero-initialised context. Not valid for scheduling until configured.
    pub const fn zero() -> Self {
        Self {
            gp_regs: [0; 10],
            frame_pointer: 0,
            link_register: 0,
            stack_pointer: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// KernelStack
// ---------------------------------------------------------------------------

/// Per-process kernel-mode execution stack.
///
/// Allocated from the kernel heap. The stack grows downward on AArch64;
/// `top` is the highest address (initial SP value for a new process).
///
/// Layout in virtual memory (addresses increasing upward):
///
///   allocation_ptr → [guard page — unmapped, 4 KiB]
///                    [stack — 64 KiB, grows downward]  ← top
///
/// The guard page causes a Data Abort on stack overflow rather than silently
/// corrupting adjacent heap memory.
///
/// Reference: Linux CONFIG_VMAP_STACK guard page behaviour.
pub struct KernelStack {
    /// Raw pointer to the heap allocation (base of guard page + stack).
    allocation_ptr: NonNull<u8>,
    /// Layout of the full allocation (guard page + stack).
    allocation_layout: Layout,
    /// Physical address of the guard page.
    ///
    /// Saved so we can remap it before handing the allocation back to the heap
    /// on `Drop` (the heap needs to write free-list metadata into the guard page).
    guard_page_phys: crate::memory::PhysicalAddress,
    /// Virtual address of the top of the stack (highest address).
    ///
    /// On AArch64, SP starts here and grows downward.
    /// Guaranteed to be 16-byte aligned (AAPCS64 requirement at EL1 entry).
    pub top: u64,
}

// SAFETY: KernelStack is owned by exactly one Process and never shared.
unsafe impl Send for KernelStack {}

impl KernelStack {
    /// Allocate a new kernel stack from the global heap, with a guard page.
    ///
    /// The heap returns 16-byte aligned memory (not necessarily page-aligned).
    /// We allocate `2 × PAGE_SIZE + KERNEL_STACK_SIZE` bytes to guarantee at
    /// least one fully page-aligned PAGE_SIZE region within the allocation,
    /// which we unmap to act as the guard page.
    ///
    /// Memory layout (addresses increasing upward):
    ///
    ///   allocation_ptr → [0–n bytes — unused alignment padding]
    ///   guard_page_va  → [PAGE_SIZE — unmapped guard page]
    ///   stack_bottom   → [KERNEL_STACK_SIZE — stack, grows downward]
    ///                  ← top (initial SP)
    ///
    /// Returns `None` if the heap or the kernel page table operation fails.
    ///
    /// # Safety
    /// Must be called with IRQs disabled (scheduler invariant).
    pub fn allocate() -> Option<Self> {
        // Allocate 2 extra pages to ensure a full page-aligned PAGE_SIZE window
        // exists within the heap block for the guard page.
        // The heap supports up to 16-byte alignment; we handle page alignment
        // ourselves within the oversized allocation.
        let total_size = 2 * KERNEL_STACK_GUARD_SIZE + KERNEL_STACK_SIZE;
        let layout = Layout::from_size_align(total_size, 16).ok()?;
        // Zero-initialise to catch uninitialised-stack reads during debugging.
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        let allocation_ptr = NonNull::new(ptr)?;

        // Find the first page-aligned address within the allocation.
        // This is the guard page.  Since the allocation is at least 2 pages
        // larger than KERNEL_STACK_SIZE, there is always room.
        let alloc_va = ptr as u64;
        let guard_va_raw = (alloc_va + KERNEL_STACK_GUARD_SIZE as u64 - 1)
            & !(KERNEL_STACK_GUARD_SIZE as u64 - 1);
        let guard_va = crate::memory::VirtualAddress::new(guard_va_raw);

        // Unmap the guard page from the kernel page table.
        // The physical page remains owned by the heap allocation.
        let guard_page_phys = unsafe {
            crate::memory::with_kernel_page_table(|page_table, _phys| {
                page_table.unmap_no_free(guard_va)
            })
        };

        let guard_page_phys = match guard_page_phys {
            Some(phys) => phys,
            None => {
                // Guard page was not mapped (unexpected).  Dealloc and fail.
                unsafe { alloc::alloc::dealloc(ptr, layout) };
                return None;
            }
        };

        // Stack occupies [guard_va + PAGE_SIZE, guard_va + PAGE_SIZE + STACK_SIZE).
        // Top (initial SP) is at the highest address, growing downward.
        let top = guard_va_raw + KERNEL_STACK_GUARD_SIZE as u64 + KERNEL_STACK_SIZE as u64;

        Some(Self {
            allocation_ptr,
            allocation_layout: layout,
            guard_page_phys,
            top,
        })
    }

    /// Virtual address of the lowest byte of the stack (above the guard page).
    pub fn bottom(&self) -> u64 {
        let alloc_va = self.allocation_ptr.as_ptr() as u64;
        let guard_va = (alloc_va + KERNEL_STACK_GUARD_SIZE as u64 - 1)
            & !(KERNEL_STACK_GUARD_SIZE as u64 - 1);
        guard_va + KERNEL_STACK_GUARD_SIZE as u64
    }

    /// Virtual address of the guard page (one page below `bottom()`).
    pub fn guard_page_va(&self) -> u64 {
        let alloc_va = self.allocation_ptr.as_ptr() as u64;
        (alloc_va + KERNEL_STACK_GUARD_SIZE as u64 - 1) & !(KERNEL_STACK_GUARD_SIZE as u64 - 1)
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        // Remap the guard page before handing the allocation back to the heap.
        // The heap writes free-list metadata starting at `allocation_ptr`;
        // the guard page lies within the allocation and must be accessible.
        let guard_va = crate::memory::VirtualAddress::new(self.guard_page_va());
        let guard_phys = self.guard_page_phys;
        unsafe {
            crate::memory::with_kernel_page_table(|page_table, phys| {
                let _ = page_table.map(
                    guard_va,
                    guard_phys,
                    crate::memory::virtual_memory::PAGE_FLAGS_KERNEL_DATA,
                    phys,
                );
            });
            alloc::alloc::dealloc(
                self.allocation_ptr.as_ptr(),
                self.allocation_layout,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

/// A schedulable entity (kernel task or user process).
///
/// Created by `ProcessPool::allocate()` in the scheduler. Stored as
/// `Box<Process>` inside the pool so the scheduler can hold raw pointers
/// to the current process without violating borrow rules.
pub struct Process {
    // --- Identity ---
    /// This process's PID (index + generation).
    pub pid: Pid,
    /// PID of the creating process; `None` for the idle process (PID 0).
    pub parent_pid: Option<Pid>,

    // --- Scheduling ---
    pub state: ProcessState,

    /// Scheduling priority as a nice value: -20 (highest) to 19 (lowest).
    ///
    /// New processes inherit the parent's nice value.
    /// The multilevel run queue in the scheduler maps this to a priority level.
    ///
    /// Reference: POSIX.1-2017 `nice(2)`, Linux `fs/exec.c`.
    pub nice: i8,

    // --- Process group / session ---
    /// Process group ID.
    ///
    /// Set to the process's own PID on creation (each process is initially
    /// the leader of its own group).  Changed by `setpgrp()` / `setpgid()`.
    ///
    /// Reference: POSIX.1-2017 "process group" definitions.
    pub pgid: u32,

    /// Session ID.
    ///
    /// Set to the process's own PID when `setsid()` is called.
    /// Inherited from parent on creation.
    pub sid: u32,

    // --- Resource limits ---
    /// Per-process resource limits (subset of RLIMIT_*).
    pub resource_limits: ResourceLimits,

    // --- File creation mask ---
    /// POSIX umask — bits to clear from the mode argument of `open(O_CREAT)`
    /// and `mkdir()`.  Default 0o022 (group/other write bits cleared).
    ///
    /// Inherited across `fork()` and `exec()`.  Changed by the `umask()` syscall.
    /// Reference: POSIX.1-2017 §4.9 (File Creation Mask).
    pub umask: u32,

    // --- Environment variables ---
    /// Process environment — list of `KEY=VALUE` strings.
    ///
    /// Inherited from the parent on `fork()`.  Replaced by `exec()` with the
    /// envp argument passed to `execve()`.  Written onto the initial stack by
    /// the ELF loader so that userspace `getenv()` / `environ` work correctly
    /// without any additional syscalls.
    ///
    /// Reference: POSIX.1-2017 §8.1 (Environment Variables).
    pub environ: alloc::vec::Vec<alloc::string::String>,

    /// Kernel-mode register context.
    ///
    /// Valid only when `state != ProcessState::Running`.
    /// When the process is running, the CPU holds the actual state.
    pub cpu_context: CpuContext,

    // --- Address space ---
    /// User-space page table (TTBR0). `None` for the idle process.
    pub page_table: Option<Box<PageTable>>,

    // --- Stacks ---
    pub kernel_stack: KernelStack,

    // --- Signals ---
    /// Pending signal bitmask. Bit N = signal N pending (N in 1..=63).
    ///
    /// Written atomically with `fetch_or(AcqRel)` from any context
    /// (including IRQ handlers). Read with `load(Acquire)` in process context.
    pub pending_signals: AtomicU64,

    /// Signal mask — blocked signals bitmask. Bit N = signal N blocked.
    ///
    /// Signals whose bit is set here are not delivered to the process.
    /// SIGKILL (9) and SIGSTOP (19) cannot be blocked (bits are ignored).
    ///
    /// Modified only from process context via `sigprocmask()`.
    pub signal_mask: u64,

    /// Per-signal disposition. Protected by the scheduler lock.
    ///
    /// signal_handlers[0] is unused (signal 0 has no meaning).
    /// signal_handlers[N] = disposition for signal N (N in 1..=63).
    pub signal_handlers: [SignalAction; SIGNAL_COUNT],

    /// Alternate signal stack (set via `sigaltstack(2)`).
    ///
    /// `None` when no alternate stack has been registered or the registered
    /// stack has been disabled with `SS_DISABLE`.
    pub signal_stack: Option<SignalStack>,

    /// True while the process is executing a signal handler on the alternate stack.
    ///
    /// Used to guard against recursive alternate-stack use: if a second signal
    /// arrives while this is true, the handler is delivered on the current
    /// (already alternate) stack rather than re-initialising the stack pointer.
    pub on_signal_stack: bool,

    // --- CPU usage tracking (for getrusage) ---
    /// Number of scheduler ticks consumed by this process in user mode.
    pub user_ticks: u64,

    // --- Exit / wait ---
    /// Exit code set by `sys_exit()`. Stored here for parent's `wait()`.
    pub exit_code: i32,

    // --- Memory ---
    /// Next available virtual address for anonymous `mmap()` allocations.
    pub mmap_next_va: u64,

    /// Tracked anonymous mmap regions (for `munmap()` and address-space cleanup).
    pub mmap_regions: Vec<MmapRegion>,

    // --- Copy-on-Write ---
    /// Pages marked read-only for CoW sharing.
    ///
    /// Key: user virtual address (page-aligned).
    /// Value: the shared physical page.
    ///
    /// On a write fault to a CoW page:
    ///   1. Allocate a new physical page.
    ///   2. Copy contents from `cow_pages[va]` into the new page.
    ///   3. Remap VA → new page with R/W flags.
    ///   4. Remove the entry from `cow_pages`.
    ///
    /// After fork(), both parent and child have their respective CoW maps
    /// (the parent keeps its own map; the child gets a copy).
    pub cow_pages: BTreeMap<u64, crate::memory::PhysicalAddress>,

    // --- Current working directory ---
    /// The inode of the current working directory.
    ///
    /// `None` until `vfs_init()` has run; set to VFS root on first schedule.
    /// Changed by `chdir()`.  Used for relative path resolution in `open()`.
    pub cwd: Option<Arc<dyn crate::fs::Inode>>,
    /// Absolute path of the current working directory as a string.
    ///
    /// Kept in sync with `cwd` by `sys_chdir`. Used by `sys_getcwd` to avoid
    /// traversing the inode tree upward.  Starts as `"/"`.
    pub cwd_path: alloc::string::String,

    // --- TTY ---
    /// True if this is the foreground process for TTY signal delivery (SIGINT, SIGTSTP).
    pub is_foreground: bool,

    // --- Process name (for prctl PR_SET_NAME and /proc/<pid>/status) ---
    /// Null-terminated process name (up to 15 characters + NUL).
    ///
    /// Set by `prctl(PR_SET_NAME, ...)`. Defaults to the empty string.
    /// Reference: Linux `prctl(2)` PR_SET_NAME.
    pub name: [u8; 16],

    // --- Thread identity ---
    /// Thread group ID — same as the PID of the thread group leader.
    ///
    /// For the main thread (and non-threaded processes): `tgid == pid.index`.
    /// For threads created by `clone(CLONE_THREAD)`: `tgid` equals the leader's
    /// `pid.index`, shared across the entire thread group.
    ///
    /// `getpid()` returns this value; `gettid()` returns `pid.index`.
    /// Reference: Linux kernel/fork.c, copy_process(), CLONE_THREAD handling.
    pub tgid: u32,

    /// TLS base address stored in `TPIDR_EL0`.
    ///
    /// Set by `clone(CLONE_SETTLS)` or `set_tls()` syscall.
    /// Written into the hardware register on each context switch in.
    /// Zero for processes that have not called set_tls (no TLS configured).
    pub tls_base: u64,

    /// True if this schedulable entity is a thread (shares TTBR0 with its
    /// thread group leader rather than owning the page table exclusively).
    ///
    /// When `is_thread == true`, `exit()` must NOT free the page table —
    /// only the thread group leader (last thread with `tgid == pid.index`)
    /// should release the address space.
    pub is_thread: bool,

    // --- Capabilities ---
    /// Process capability bitmask.
    ///
    /// Each bit grants access to a privileged kernel resource.  A process may
    /// only grant capabilities it already holds to children via `sys_spawn`.
    /// bzinit (PID 1) is granted `CAP_ALL` at boot and distributes individual
    /// bits to services via the `capabilities` field in `.service` files.
    ///
    /// See `capability` constants below.
    pub capabilities: u64,

    // --- POSIX alarm ---
    /// Scheduler-tick deadline for SIGALRM delivery.
    ///
    /// 0 = no alarm set.  Set by `alarm(seconds)` to
    /// `current_tick() + seconds * TICKS_PER_SECOND`.  The CPU-0 scheduler
    /// tick handler scans this field and delivers SIGALRM when the deadline
    /// passes, then resets it to 0.
    ///
    /// Reference: POSIX.1-2017 `alarm(2)`.
    pub alarm_deadline_tick: u64,

    // --- File descriptors ---
    /// Shared file descriptor table.
    ///
    /// Wrapped in `Arc<SpinLock<...>>` so all threads in a thread group share
    /// the same table (POSIX requirement).  `fork()` creates a new Arc with a
    /// deep copy; `clone(CLONE_THREAD)` clones the Arc pointer.
    ///
    /// Slot 0 = stdin (TTY), 1 = stdout (TTY), 2 = stderr (TTY) by convention.
    pub file_descriptor_table: Arc<SpinLock<FileDescriptorTable>>,
}

impl Process {
    /// Allocate and initialise a new process struct.
    ///
    /// The process is created in `ProcessState::Ready` with:
    ///   - All signals pending = 0 (no pending signals).
    ///   - All signal handlers = `SignalAction::Default`.
    ///   - `cpu_context` zeroed — the caller must configure it before scheduling.
    ///   - No user page table — the caller must install one for EL0 processes.
    ///
    /// Returns `None` if the kernel stack allocation fails (OOM).
    pub fn new(pid: Pid, parent_pid: Option<Pid>) -> Option<Self> {
        let kernel_stack = KernelStack::allocate()?;
        let pid_u32 = pid.index as u32;
        Some(Self {
            pid,
            parent_pid,
            state: ProcessState::Ready,
            nice: NICE_DEFAULT,
            pgid: pid_u32,
            sid: pid_u32,
            resource_limits: ResourceLimits::default_limits(),
            umask: 0o022,
            environ: alloc::vec::Vec::new(),
            cpu_context: CpuContext::zero(),
            page_table: None,
            kernel_stack,
            pending_signals: AtomicU64::new(0),
            signal_mask: 0,
            signal_handlers: [SignalAction::Default; SIGNAL_COUNT],
            signal_stack: None,
            on_signal_stack: false,
            user_ticks: 0,
            exit_code: 0,
            mmap_next_va: MMAP_USER_BASE,
            mmap_regions: Vec::new(),
            cow_pages: BTreeMap::new(),
            cwd: None,
            cwd_path: alloc::string::String::from("/"),
            is_foreground: false,
            name: [0u8; 16],
            tgid: pid_u32,
            tls_base: 0,
            is_thread: false,
            capabilities: 0,
            alarm_deadline_tick: 0,
            file_descriptor_table: Arc::new(SpinLock::new(FileDescriptorTable::new_with_tty())),
        })
    }

    // -----------------------------------------------------------------------
    // Signal helpers
    // -----------------------------------------------------------------------

    /// Deliver signal `signum` to this process.
    ///
    /// May be called from IRQ context. Sets bit `signum` in `pending_signals`
    /// atomically. The signal is delivered (handler called) the next time this
    /// process returns from a syscall or exception.
    ///
    /// Returns `false` if `signum` is out of range [1, SIGNAL_COUNT).
    pub fn send_signal(&self, signum: u8) -> bool {
        if signum == 0 || signum as usize >= SIGNAL_COUNT {
            return false;
        }
        self.pending_signals.fetch_or(1u64 << signum, Ordering::AcqRel);
        true
    }

    /// Pop the lowest-numbered pending signal, clearing its pending bit.
    ///
    /// Returns `None` if no signals are pending.
    ///
    /// Must be called from process context only (not from IRQ handlers).
    /// The load+clear is not a single atomic step; this is safe only when
    /// no other code can concurrently call `take_pending_signal` for the
    /// same process (enforced by the scheduler: only the running process
    /// calls this method).
    pub fn take_pending_signal(&self) -> Option<u8> {
        let pending = self.pending_signals.load(Ordering::Acquire);
        if pending == 0 {
            return None;
        }
        let signum = pending.trailing_zeros() as u8;
        self.pending_signals
            .fetch_and(!(1u64 << signum), Ordering::AcqRel);
        Some(signum)
    }

    /// Register a signal disposition.
    ///
    /// Validates that `signum` is in range [1, SIGNAL_COUNT) and that any
    /// handler VA is below `USER_ADDR_LIMIT`.
    pub fn set_signal_handler(
        &mut self,
        signum: u8,
        action: SignalAction,
    ) -> Result<(), SignalError> {
        if signum == 0 || signum as usize >= SIGNAL_COUNT {
            return Err(SignalError::InvalidSignalNumber);
        }
        if let SignalAction::Handler { va: virtual_address, .. } = action {
            if virtual_address >= USER_ADDR_LIMIT {
                return Err(SignalError::HandlerInKernelSpace);
            }
        }
        self.signal_handlers[signum as usize] = action;
        Ok(())
    }

    /// Return the entry point VA of the signal trampoline for this process.
    ///
    /// The trampoline is a small code stub mapped into the user address space.
    /// After the signal handler returns, it executes `svc SYSCALL_SIGRETURN`
    /// to restore the pre-signal context.
    ///
    /// The trampoline VA is stored per-process to support ASLR in Fase 5.
    /// For Fase 4, all processes share the same fixed VA.
    pub fn signal_trampoline_va(&self) -> u64 {
        // Fixed signal trampoline VA for Fase 4.
        // TECHNICAL DEBT (Fase 5): randomise per process (ASLR).
        SIGNAL_TRAMPOLINE_VA
    }
}

/// Fixed virtual address of the signal return trampoline.
///
/// Mapped into every user process address space by the ELF loader.
/// The trampoline executes `svc #SYSCALL_SIGRETURN` to restore the process
/// context after a signal handler returns.
///
/// TECHNICAL DEBT (Fase 5): apply ASLR to this address.
pub const SIGNAL_TRAMPOLINE_VA: u64 = 0x0000_0000_0000_2000;

/// AArch64 instruction encoding for `svc #23` (sigreturn syscall).
///
/// Sigreturn is syscall 23 in our ABI.
/// Encoding: `svc #imm16` = 0xD400_0001 | (imm16 << 5)
/// svc #23 = 0xD400_0001 | (23 << 5) = 0xD400_02E1
pub const SIGNAL_TRAMPOLINE_INSTRUCTION: u32 = 0xD400_02E1;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalError {
    /// Signal number is 0 or >= SIGNAL_COUNT.
    InvalidSignalNumber,
    /// Handler virtual address is in kernel space (>= USER_ADDR_LIMIT).
    HandlerInKernelSpace,
}

// ---------------------------------------------------------------------------
// Context switch — extern "C" symbols from process/context_switch.S
// ---------------------------------------------------------------------------

extern "C" {
    /// Switch CPU context from `from` to `to`.
    ///
    /// Saves the callee-saved registers (x19–x30) and SP of the process
    /// whose context is at `*from`, then restores them from `*to`, and
    /// returns into the saved link register (x30) of `to`.
    ///
    /// # Safety
    /// - Both pointers must be valid, 8-byte-aligned `CpuContext` structs.
    /// - Must be called with IRQs disabled (DAIF.I = 1).
    /// - `from` must be the currently running process's `cpu_context`.
    /// - `to` must have a valid `stack_pointer` and `link_register`.
    /// - The caller must not assume any caller-saved registers survive
    ///   across this call (the CPU will be running `to`'s code after `ret`).
    pub fn context_switch(from: *mut CpuContext, to: *const CpuContext);

    /// Entry trampoline for new EL0 processes.
    ///
    /// Called as the first "return address" from `context_switch` for a
    /// freshly created user process. SP must point to an `ExceptionFrame`
    /// placed at the top of the kernel stack by `fork()` or `exec()`.
    ///
    /// Restores all GPRs, ELR_EL1, SPSR_EL1, and SP_EL0 from the frame,
    /// then executes `eret` to enter EL0.
    ///
    /// Never returns to the kernel (it erets to user space).
    pub fn process_entry_trampoline_el0();
}
