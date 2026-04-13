// scheduler/mod.rs — Round-robin process scheduler for Bazzulto OS.
//
// Design:
//   - ProcessPool: a Vec<ProcessSlot> that grows up to PID_MAX = 32768 slots.
//     Each slot is either Empty or Occupied(Box<Process>). Slot 0 is the idle
//     process (always occupied, never reused).
//   - RunQueue: a VecDeque<Pid> for O(1) push_back/pop_front.
//   - Scheduling policy: round-robin, non-preemptive within the kernel.
//     The timer IRQ calls `schedule()` to yield to the next ready process.
//   - Single global SCHEDULER protected by interrupt disabling (single-core).
//
// Linux reference:
//   Completely Fair Scheduler (CFS) — vruntime, red-black tree run queue.
//   We implement a simpler O(1) round-robin as a correct baseline.
//
// Locking:
//   Interrupts must be disabled (DAIF.I = 1) before accessing the scheduler.
//   On SMP this becomes a spinlock; for now the invariant is documented and
//   enforced with a debug_assert on the DAIF register.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::arm64::exceptions::ExceptionFrame;
use crate::process::{
    context_switch, process_entry_trampoline_el0, CpuContext, MmapRegion, Pid,
    Process, ProcessState, ResourceLimits, SignalAction,
    MMAP_MAX_REGIONS, NICE_DEFAULT, PRIORITY_LEVELS, SIGNAL_COUNT,
    nice_to_priority_level,
};
use crate::smp::{self, MAX_CPUS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of processes, matching Linux PID_MAX_DEFAULT.
///
/// Reference: Linux kernel/pid.h PID_MAX_DEFAULT = 32768.
/// macOS uses ~99999; Windows ~65536. We match Linux's conservative default.
pub const PID_MAX: usize = 32768;

// ---------------------------------------------------------------------------
// ProcessSlot
// ---------------------------------------------------------------------------

enum ProcessSlot {
    /// This slot in the pool is unoccupied and available for allocation.
    Empty,
    /// The slot holds a live (or zombie) process.
    Occupied(Box<Process>),
}

// ---------------------------------------------------------------------------
// ProcessPool
// ---------------------------------------------------------------------------

/// Sparse pool of all processes indexed by PID slot.
///
/// Grows lazily from 0 to PID_MAX entries. Slot 0 is pre-populated by
/// `ProcessPool::init()` with the idle process and never reused.
///
/// PID allocation strategy:
///   - Linear scan starting from `next_search_index` (wraps at PID_MAX).
///   - This matches the POSIX requirement that PIDs are not immediately
///     reused, and the generation counter in Pid provides ABA safety.
struct ProcessPool {
    slots: Vec<ProcessSlot>,
    /// Next slot index to check during PID allocation (not the next PID).
    next_search_index: usize,
    /// Total processes currently alive (not Zombie or Empty).
    alive_count: usize,
}

impl ProcessPool {
    const fn empty() -> Self {
        Self {
            slots: Vec::new(),
            next_search_index: 1, // skip idle (slot 0)
            alive_count: 0,
        }
    }

    /// Populate slot 0 with the idle process.
    ///
    /// Must be called exactly once during scheduler init.
    fn init_idle(&mut self) -> Option<()> {
        let idle_pid = Pid::IDLE;
        let mut idle = Process::new(idle_pid, None)?;
        idle.state = ProcessState::Running; // idle starts as the running process
        self.slots.push(ProcessSlot::Occupied(Box::new(idle)));
        self.alive_count += 1;
        Some(())
    }

    /// Allocate the next free slot and return a Pid for it.
    ///
    /// Grows `slots` if necessary. Returns `None` if the pool is at PID_MAX.
    fn allocate_slot(&mut self) -> Option<Pid> {
        let pool_len = self.slots.len();

        // Search from next_search_index forward (wrapping).
        for offset in 0..PID_MAX {
            let index = (self.next_search_index + offset) % PID_MAX;

            if index == 0 {
                continue; // slot 0 is the idle process — never reallocated
            }

            if index < pool_len {
                if matches!(self.slots[index], ProcessSlot::Empty) {
                    self.next_search_index = (index + 1) % PID_MAX;
                    // Generation is stored per-slot so that a reused index
                    // gets a different generation. We derive it from the
                    // previous occupant, but since the slot is Empty now we
                    // just use 0 — the caller embeds the generation in the Pid.
                    // For simplicity in Fase 4: generation always starts at 1
                    // (0 is reserved for the idle process).
                    return Some(Pid::new(index as u16, 1));
                }
            } else {
                // Grow the pool (lazy allocation).
                if index >= PID_MAX {
                    return None;
                }
                // Fill any gap between pool_len and index with Empty slots.
                while self.slots.len() <= index {
                    self.slots.push(ProcessSlot::Empty);
                }
                self.next_search_index = (index + 1) % PID_MAX;
                return Some(Pid::new(index as u16, 1));
            }
        }

        None // pool is full
    }

    /// Place `process` into its slot (slot must currently be Empty or growing).
    fn insert(&mut self, process: Box<Process>) {
        let index = process.pid.index as usize;
        while self.slots.len() <= index {
            self.slots.push(ProcessSlot::Empty);
        }
        self.slots[index] = ProcessSlot::Occupied(process);
        self.alive_count += 1;
    }

    /// Get a shared reference to a process by Pid.
    ///
    /// Returns `None` if the slot is empty or the generation does not match.
    fn get(&self, pid: Pid) -> Option<&Process> {
        let index = pid.index as usize;
        match self.slots.get(index)? {
            ProcessSlot::Occupied(process) if process.pid == pid => Some(process),
            _ => None,
        }
    }

    /// Get a mutable reference to a process by Pid.
    fn get_mut(&mut self, pid: Pid) -> Option<&mut Process> {
        let index = pid.index as usize;
        match self.slots.get_mut(index)? {
            ProcessSlot::Occupied(process) if process.pid == pid => Some(process),
            _ => None,
        }
    }

    /// Get a shared reference to a process by slot index (raw u16 index, not Pid).
    ///
    /// Used by `ProcPidDirInode::lookup("comm")` to read process names for /proc.
    pub fn get_by_index(&self, index: usize) -> Option<&Process> {
        match self.slots.get(index)? {
            ProcessSlot::Occupied(process) => Some(process),
            _ => None,
        }
    }

    /// Release the slot occupied by `pid`, freeing all its resources.
    ///
    /// The `Box<Process>` is dropped here, which frees the kernel stack and
    /// page table. Returns the process's exit code.
    fn free_slot(&mut self, pid: Pid) -> Option<i32> {
        let index = pid.index as usize;
        let slot = self.slots.get_mut(index)?;
        if let ProcessSlot::Occupied(process) = slot {
            if process.pid != pid {
                return None;
            }
            let exit_code = process.exit_code;
            *slot = ProcessSlot::Empty;
            self.alive_count = self.alive_count.saturating_sub(1);
            Some(exit_code)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// RunQueue — multilevel priority queue
// ---------------------------------------------------------------------------

/// Multilevel round-robin run queue with 40 priority levels.
///
/// Level 0 = nice -20 (highest priority), level 39 = nice 19 (lowest).
/// Within each level, scheduling is FIFO (first-in, first-out) for fairness.
///
/// The scheduler always picks the highest non-empty level (lowest level index).
/// This matches Linux's O(1) scheduler concept before CFS replaced it.
///
/// Reference: Linux kernel/sched/rt.c (multilevel priority bitmap concept).
struct RunQueue {
    /// Per-level FIFO queues.
    levels: [VecDeque<Pid>; PRIORITY_LEVELS],
    /// Bitmap of non-empty levels. Bit N = 1 means level N has ready PIDs.
    ///
    /// Using a u64 — sufficient for 40 levels (bits 0–39).
    non_empty_mask: u64,
}

impl RunQueue {
    fn empty() -> Self {
        // VecDeque is not Copy, so we cannot use array initializer syntax.
        // Initialize with a fixed-size array using Default.
        Self {
            levels: core::array::from_fn(|_| VecDeque::new()),
            non_empty_mask: 0,
        }
    }

    /// Enqueue `pid` at the priority level derived from `nice`.
    fn push_with_nice(&mut self, pid: Pid, nice: i8) {
        let level = nice_to_priority_level(nice);
        self.levels[level].push_back(pid);
        self.non_empty_mask |= 1u64 << level;
    }

    /// Enqueue `pid` at the default priority level (nice 0).
    fn push(&mut self, pid: Pid) {
        self.push_with_nice(pid, NICE_DEFAULT);
    }

    /// Dequeue the highest-priority ready PID (lowest level index).
    fn pop(&mut self) -> Option<Pid> {
        // Find the lowest set bit = highest priority non-empty level.
        if self.non_empty_mask == 0 {
            return None;
        }
        let level = self.non_empty_mask.trailing_zeros() as usize;
        let pid = self.levels[level].pop_front();
        if self.levels[level].is_empty() {
            self.non_empty_mask &= !(1u64 << level);
        }
        pid
    }

    /// Total number of PIDs across all priority levels.
    fn total_len(&self) -> usize {
        self.levels.iter().map(|level_queue| level_queue.len()).sum()
    }

    /// Pop from the lowest-priority non-empty level (highest level index).
    ///
    /// Used by work stealing to take work that is least likely to be
    /// cache-hot on the victim CPU.
    fn pop_lowest_priority(&mut self) -> Option<Pid> {
        // Scan from the highest level index downward.
        for level_index in (0..PRIORITY_LEVELS).rev() {
            if !self.levels[level_index].is_empty() {
                let pid = self.levels[level_index].pop_front();
                if self.levels[level_index].is_empty() {
                    self.non_empty_mask &= !(1u64 << level_index);
                }
                return pid;
            }
        }
        None
    }

    /// Remove `pid` from whichever level it is in (O(n) scan, used rarely).
    fn remove(&mut self, pid: Pid) {
        for (level_index, level_queue) in self.levels.iter_mut().enumerate() {
            let before = level_queue.len();
            level_queue.retain(|entry| *entry != pid);
            if level_queue.len() < before && level_queue.is_empty() {
                self.non_empty_mask &= !(1u64 << level_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub struct Scheduler {
    pool: ProcessPool,
    /// Per-CPU run queues.
    ///
    /// Each CPU core dequeues from its own slot and, when empty, steals from
    /// the busiest other core's queue (work stealing).
    run_queues: [RunQueue; MAX_CPUS],
    /// PID currently executing on each CPU core.
    ///
    /// Indexed by cpu_id. Invariant: the named process has state Running.
    current_pids: [Pid; MAX_CPUS],
    /// PID of the system init process (bzinit, PID 1).
    ///
    /// Orphaned processes are reparented to this process so their zombie
    /// slots are reaped when bzinit calls wait(-1) in its supervisor loop.
    /// Set by `set_init_pid()` after bzinit is spawned.
    init_pid: Option<Pid>,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            pool: ProcessPool::empty(),
            run_queues: core::array::from_fn(|_| RunQueue::empty()),
            current_pids: [Pid::IDLE; MAX_CPUS],
            init_pid: None,
        }
    }

    /// Return the cpu_id of the calling core, clamped to [0, MAX_CPUS).
    ///
    /// If TPIDR_EL1 has not been set yet (early boot, before per_cpu_init),
    /// `current_cpu()` may return a zeroed struct — cpu_id 0. The `.min` clamp
    /// ensures we never index out of bounds regardless.
    fn current_cpu_id() -> usize {
        // SAFETY: We only read cpu_id from the PerCpuData; even if TPIDR_EL1
        // is zero early in boot, the worst case is cpu_id 0, which is valid.
        let raw_cpu_id = unsafe { smp::current_cpu() }.cpu_id as usize;
        raw_cpu_id.min(MAX_CPUS - 1)
    }

    /// Register `pid` as the system init process (bzinit).
    ///
    /// Must be called once, immediately after bzinit is spawned at boot.
    /// After this call, orphaned processes are reparented to `pid` rather
    /// than being freed immediately.
    pub fn set_init_pid(&mut self, pid: Pid) {
        self.init_pid = Some(pid);
    }

    /// Initialise the scheduler with the idle process.
    ///
    /// Must be called before any other scheduler function.
    /// The idle process is a minimal kernel task (no user address space)
    /// that runs when no other process is ready.  It executes `wfe` in a loop.
    ///
    /// Returns `None` if the kernel heap cannot satisfy the idle process allocation.
    fn init(&mut self) -> Option<()> {
        self.pool.init_idle()?;
        // All CPUs start on the idle process.
        self.current_pids = [Pid::IDLE; MAX_CPUS];
        Some(())
    }

    // -----------------------------------------------------------------------
    // Process creation
    // -----------------------------------------------------------------------

    /// Allocate a new process slot and create a process struct.
    ///
    /// The process is not yet ready to run — the caller must:
    ///   1. Install a user page table (`process.page_table = Some(pt)`).
    ///   2. Set up the kernel stack (place ExceptionFrame, configure CpuContext).
    ///   3. Call `make_ready(pid)` to enqueue the process.
    ///
    /// Returns `None` if PID_MAX is reached or the heap is out of memory.
    pub fn create_process(&mut self, parent_pid: Option<Pid>) -> Option<Pid> {
        let pid = self.pool.allocate_slot()?;
        let process = Box::new(Process::new(pid, parent_pid)?);
        let pid = process.pid;
        self.pool.insert(process);
        Some(pid)
    }

    /// Move a process from Ready (not enqueued) to Ready (enqueued in run queue).
    ///
    /// Uses load-balanced placement: the process goes to the CPU whose run
    /// queue is currently shortest.
    pub fn make_ready(&mut self, pid: Pid) {
        if let Some(process) = self.pool.get_mut(pid) {
            let nice = process.nice;
            process.state = ProcessState::Ready;
            self.enqueue_load_balanced_with_nice(pid, nice);
        }
    }

    /// Enqueue `pid` on the least-loaded CPU's run queue.
    fn enqueue_load_balanced_with_nice(&mut self, pid: Pid, nice: i8) {
        let target_cpu = (0..MAX_CPUS)
            .min_by_key(|&cpu_index| self.run_queues[cpu_index].total_len())
            .unwrap_or(0);
        self.run_queues[target_cpu].push_with_nice(pid, nice);
    }

    // -----------------------------------------------------------------------
    // Scheduling
    // -----------------------------------------------------------------------

    /// Schedule on the current CPU.
    ///
    /// Convenience wrapper for call sites that do not need to specify the
    /// CPU explicitly (syscall handlers, IRQ handlers).
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn schedule(&mut self) {
        let cpu_id = Self::current_cpu_id();
        self.schedule_on_core(cpu_id);
    }

    /// Select the next process on `cpu_id` and switch to it.
    ///
    /// Called from the timer IRQ handler (with IRQs disabled) and from
    /// blocking syscalls (yield, wait, read on empty pipe).
    ///
    /// Behaviour:
    ///   1. Re-enqueue the current process for this core if still Running.
    ///   2. CPU 0 wakes sleeping processes (one designated waker avoids races).
    ///   3. Pop from this CPU's local run queue.
    ///   4. If empty, attempt work stealing from the busiest other queue.
    ///   5. Fall back to the idle process if nothing is available.
    ///   6. Perform a context switch into the selected process.
    ///
    /// # Safety
    /// Must be called with IRQs disabled. The caller is responsible for
    /// re-enabling IRQs after this returns (which may be in the context of
    /// a different process).
    pub unsafe fn schedule_on_core(&mut self, cpu_id: usize) {
        let from_pid = self.current_pids[cpu_id];

        // CPU 0 is responsible for waking sleeping processes.
        //
        // Having a single designated waker avoids a race where multiple cores
        // simultaneously scan the pool and each enqueue the same PID.
        if cpu_id == 0 {
            let now_tick = crate::platform::qemu_virt::timer::current_tick();
            let mut wakeup_pids: alloc::vec::Vec<Pid> = alloc::vec::Vec::new();
            for slot in &self.pool.slots {
                if let ProcessSlot::Occupied(process) = slot {
                    if let ProcessState::Sleeping { wake_at_tick } = process.state {
                        if now_tick >= wake_at_tick {
                            wakeup_pids.push(process.pid);
                        }
                    }
                }
            }
            for pid in wakeup_pids {
                if let Some(process) = self.pool.get_mut(pid) {
                    let nice = process.nice;
                    process.state = ProcessState::Ready;
                    self.enqueue_load_balanced_with_nice(pid, nice);
                }
            }

            // Deliver SIGALRM to any process whose alarm deadline has passed.
            //
            // CPU 0 is the designated SIGALRM dispatcher — same reasoning as the
            // sleep-wake logic above: a single scanner avoids duplicate delivery.
            //
            // Reference: POSIX.1-2017 `alarm(2)`.
            let mut alarm_pids: alloc::vec::Vec<Pid> = alloc::vec::Vec::new();
            for slot in &self.pool.slots {
                if let ProcessSlot::Occupied(process) = slot {
                    if process.alarm_deadline_tick != 0
                        && now_tick >= process.alarm_deadline_tick
                    {
                        alarm_pids.push(process.pid);
                    }
                }
            }
            for pid in alarm_pids {
                if let Some(process) = self.pool.get_mut(pid) {
                    process.alarm_deadline_tick = 0;
                    process.send_signal(14); // SIGALRM = 14
                }
            }
        }

        // Re-enqueue the current process on its local queue if still runnable.
        if let Some(current) = self.pool.get_mut(from_pid) {
            if current.state == ProcessState::Running {
                let nice = current.nice;
                current.state = ProcessState::Ready;
                if from_pid != Pid::IDLE {
                    self.run_queues[cpu_id].push_with_nice(from_pid, nice);
                }
            }
            // Blocked / Waiting / Sleeping / Stopped / Zombie: do not re-enqueue.
        }

        // Pick the next process: local queue → steal → idle.
        let to_pid = self.run_queues[cpu_id]
            .pop()
            .or_else(|| self.steal_work(cpu_id))
            .unwrap_or(Pid::IDLE);

        if to_pid == from_pid {
            // Only one runnable process: restore state and return without switching.
            if let Some(process) = self.pool.get_mut(from_pid) {
                process.state = ProcessState::Running;
            }
            return;
        }

        // Activate the new process's address space (TTBR0) and restore TLS.
        let next_tls_base: u64 = if let Some(next) = self.pool.get_mut(to_pid) {
            next.state = ProcessState::Running;
            let tls = next.tls_base;
            if let Some(page_table) = &next.page_table {
                // Normal user process: load its own page table root.
                page_table.activate_el0();
            } else if next.is_thread {
                // Thread: shares the parent's address space.
                // Do NOT clear TTBR0 — the existing register value is the
                // shared page table root and must remain active.
                // The TLB flush on context_switch is sufficient for coherency
                // because threads share the same ASID/PA root.
            } else {
                // Kernel-only process (idle): clear TTBR0_EL1.
                clear_ttbr0();
            }
            tls
        } else {
            0
        };

        self.current_pids[cpu_id] = to_pid;

        // Restore TLS base for the incoming process/thread.
        //
        // Writing TPIDR_EL0 here ensures the correct TLS pointer is visible in
        // user space from the very first instruction after eret.
        //
        // Reference: ARM ARM DDI 0487 D13.2.116 "TPIDR_EL0".
        // SAFETY: TPIDR_EL0 is a user-opaque register; writing it from EL1 is
        // always safe. IRQs are disabled at this point.
        core::arch::asm!(
            "msr tpidr_el0, {tls}",
            tls = in(reg) next_tls_base,
            options(nostack, nomem),
        );

        // Perform the actual context switch.
        //
        // We obtain raw pointers to CpuContext inside the pool.  The pool Vec
        // does not move during context_switch (no allocation can happen while
        // IRQs are off and we hold the scheduler state).  The from/to pointers
        // are into two distinct elements.
        let from_ctx: *mut CpuContext = {
            match self.pool.get_mut(from_pid) {
                Some(process) => &mut process.cpu_context as *mut CpuContext,
                None => {
                    crate::drivers::uart::puts("[sched] schedule_on_core: from_pid not in pool idx=");
                    crate::drivers::uart::put_hex(from_pid.index as u64);
                    crate::drivers::uart::puts("\r\n");
                    panic!("schedule_on_core: from_pid not in pool");
                }
            }
        };
        let to_ctx: *const CpuContext = {
            match self.pool.get(to_pid) {
                Some(process) => &process.cpu_context as *const CpuContext,
                None => {
                    crate::drivers::uart::puts("[sched] schedule_on_core: to_pid not in pool idx=");
                    crate::drivers::uart::put_hex(to_pid.index as u64);
                    crate::drivers::uart::puts("\r\n");
                    panic!("schedule_on_core: to_pid not in pool");
                }
            }
        };

        // Release the scheduler spinlock BEFORE the context switch.
        //
        // context_switch() does not return immediately on this stack — it
        // returns later when another core (or a timer IRQ) switches back to
        // the current process.  If we kept the lock across the switch, the
        // lock would remain held from this core's perspective until the
        // process resumes, blocking all other cores from scheduling.
        scheduler_raw_lock_release();

        context_switch(from_ctx, to_ctx);

        // Reacquire the lock after returning from context_switch.
        //
        // Execution resumes here when this process is scheduled back in by
        // any core.  We re-take the lock so that the invariant "lock is held
        // while inside with_scheduler" holds when with_scheduler's epilogue
        // runs after we return.
        scheduler_raw_lock_acquire();

        // After context_switch returns we are running in a (potentially
        // different) process.  Do not access local variables below this line
        // that were live before the switch — the stack may have changed.
    }

    /// Attempt to steal work from the busiest other CPU's run queue.
    ///
    /// Finds the CPU with the most ready processes and, if it has more than
    /// one, steals the lowest-priority task from it (least cache-hot work).
    /// Returns `None` if no work is available to steal.
    fn steal_work(&mut self, thief_cpu_id: usize) -> Option<Pid> {
        let mut best_cpu: Option<usize> = None;
        let mut best_len: usize = 0;

        for other_cpu in 0..MAX_CPUS {
            if other_cpu == thief_cpu_id {
                continue;
            }
            let queue_len = self.run_queues[other_cpu].total_len();
            if queue_len > best_len {
                best_len = queue_len;
                best_cpu = Some(other_cpu);
            }
        }

        if let Some(source_cpu) = best_cpu {
            // Steal if the victim has any task.  The caller already checked
            // that the local queue is empty (pop() returned None), so migration
            // is necessary to avoid starvation on single-core or when CPUs are
            // idle.  The "> 1" threshold from classic work-stealing literature
            // only applies when the local queue is non-empty; here it is empty.
            if best_len > 0 {
                return self.run_queues[source_cpu].pop_lowest_priority();
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // fork()
    // -----------------------------------------------------------------------

    /// Clone the current process into a new child.
    ///
    /// Called from `sys_fork()`. The child receives a deep copy of the parent's
    /// user address space. Both parent and child return from fork():
    ///   - Parent returns the child's PID (> 0).
    ///   - Child returns 0 (set in the ExceptionFrame copy on the kernel stack).
    ///
    /// The child's kernel-stack frame is a copy of the parent's ExceptionFrame
    /// at the moment of the syscall, with x0 (return value) set to 0.
    ///
    /// Returns the child Pid on success, or a `ForkError` on failure.
    ///
    /// # Safety
    /// Must be called from process context with IRQs disabled.
    pub unsafe fn fork(&mut self, parent_frame: *const ExceptionFrame) -> Result<Pid, ForkError> {
        let cpu_id = Self::current_cpu_id();
        let parent_pid = self.current_pids[cpu_id];

        // Allocate the child slot.
        let child_pid = self.create_process(Some(parent_pid)).ok_or(ForkError::OutOfPids)?;

        // CoW fork: create child page table sharing physical pages with parent.
        //
        // `cow_copy_user` marks all user R/W pages in the parent as read-only
        // (with the PAGE_COW software bit) and builds the child's page table
        // pointing to the same physical pages.  On the first write by either
        // process, the page fault handler copies the page and makes it private.
        let child_page_table = {
            let parent = self.pool.get_mut(parent_pid).ok_or(ForkError::InternalError)?;
            match parent.page_table.as_mut() {
                Some(parent_table) => {
                    let child_table = crate::memory::with_physical_allocator(|phys| {
                        PageTable::cow_copy_user(parent_table, phys)
                    })
                    .map_err(|_| ForkError::OutOfMemory)?;
                    Some(Box::new(child_table))
                }
                None => None,
            }
        };

        // Build CoW page maps for parent and child.
        //
        // `collect_cow_pages` walks the page table for entries with PAGE_COW set.
        // This is called OUTSIDE `with_physical_allocator` so that the callback
        // may use the heap (BTreeMap::insert → Vec alloc).
        {
            let parent = self.pool.get_mut(parent_pid).ok_or(ForkError::InternalError)?;
            if let Some(parent_table) = parent.page_table.as_ref() {
                parent_table.collect_cow_pages(&mut |va, phys| {
                    parent.cow_pages.insert(va, phys);
                });
            }
        }

        // Copy mmap state, signal handlers, FD table, cwd, environ, umask,
        // and signal stack from parent to child.
        let (
            child_mmap_next_va,
            child_mmap_regions,
            child_signal_handlers,
            child_fd_table_arc,
            child_cwd,
            child_cwd_path,
            child_environ,
            child_umask,
            child_signal_stack,
        ) = {
            let parent = self.pool.get(parent_pid).ok_or(ForkError::InternalError)?;
            let regions: alloc::vec::Vec<MmapRegion> = parent.mmap_regions
                .iter()
                .copied()
                .collect();
            // fork(): deep-clone into an independent FD table (POSIX fork semantics).
            let fd_table_clone = {
                let guard = parent.file_descriptor_table.lock();
                guard.clone_for_fork()
            };
            let child_fd_table_arc = alloc::sync::Arc::new(crate::sync::SpinLock::new(fd_table_clone));
            let cwd = parent.cwd.clone();
            let cwd_path = parent.cwd_path.clone();
            // POSIX: environ and umask are inherited across fork().
            let environ = parent.environ.clone();
            let umask = parent.umask;
            // signal_stack is inherited; on_signal_stack is NOT (child starts fresh).
            let signal_stack = parent.signal_stack;
            (parent.mmap_next_va, regions, parent.signal_handlers, child_fd_table_arc, cwd, cwd_path, environ, umask, signal_stack)
        };

        // Build the child's initial kernel stack frame (copy of parent's ExceptionFrame
        // with x0 = 0 for the child's return value from fork).
        let child = self.pool.get_mut(child_pid).ok_or(ForkError::InternalError)?;

        child.page_table = child_page_table;

        // Build CoW map for child.
        if let Some(child_table) = child.page_table.as_ref() {
            child_table.collect_cow_pages(&mut |va, phys| {
                child.cow_pages.insert(va, phys);
            });
        }
        child.mmap_next_va = child_mmap_next_va;
        child.mmap_regions = child_mmap_regions;
        child.signal_handlers = child_signal_handlers;
        child.file_descriptor_table = child_fd_table_arc;
        child.cwd = child_cwd;
        child.cwd_path = child_cwd_path;
        child.environ = child_environ;
        child.umask = child_umask;
        child.signal_stack = child_signal_stack;
        child.on_signal_stack = false; // child starts outside the alt stack
        // POSIX: pending alarm is not inherited — child starts with no alarm.
        child.alarm_deadline_tick = 0;

        // Place the ExceptionFrame on the child's kernel stack.
        // The frame is a copy of the parent's syscall entry frame, with x0 = 0.
        let frame_dst = (child.kernel_stack.top as usize
            - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
        core::ptr::copy_nonoverlapping(parent_frame, frame_dst, 1);
        // Child's fork() returns 0.
        (*frame_dst).x[0] = 0;

        // Configure the child's CpuContext:
        //   - stack_pointer = frame_dst (process_entry_trampoline_el0 restores from here)
        //   - link_register = process_entry_trampoline_el0
        child.cpu_context.stack_pointer = frame_dst as u64;
        child.cpu_context.link_register = process_entry_trampoline_el0 as u64;

        // Enqueue the child on the least-loaded CPU.
        let child_pid = child.pid;
        let child_nice = child.nice;
        child.state = ProcessState::Ready;
        self.enqueue_load_balanced_with_nice(child_pid, child_nice);

        Ok(child_pid)
    }

    // -----------------------------------------------------------------------
    // clone_thread() — POSIX thread creation (CLONE_VM | CLONE_THREAD)
    // -----------------------------------------------------------------------

    /// Create a new thread that shares the current process's address space.
    ///
    /// Unlike `fork()`, no copy of the page table is made. Both parent and child
    /// point to the same physical TTBR0 root. The child gets:
    ///   - Its own `KernelStack` and `ExceptionFrame`.
    ///   - SP_EL0 = `child_stack` (user-space stack provided by the caller).
    ///   - `tls_base` = `tls` (written into `TPIDR_EL0` on first context switch).
    ///   - `x0 = 0` in the exception frame (thread-create return convention).
    ///   - `is_thread = true` so that `exit()` will not free the page table.
    ///   - `tgid` = parent's `tgid`.
    ///
    /// The parent returns the child's TID.
    ///
    /// # Page table ownership
    ///
    /// `Process.page_table` is a `Box<PageTable>`, which implies exclusive
    /// ownership.  For threads this would be incorrect — we cannot `clone()` a
    /// `Box`.  Instead the child's `page_table` is set to `None` and the child
    /// re-uses the parent's physical TTBR0 root address directly: when the child
    /// is scheduled, `activate_el0()` is not called (because `page_table` is
    /// `None`).  The scheduler's context-switch path already handles `None` for
    /// kernel-only processes by calling `clear_ttbr0()`, but for threads we
    /// must *not* clear TTBR0 — instead we leave the existing TTBR0 register
    /// value intact (valid because the parent and child share the same PA root).
    ///
    /// TECHNICAL DEBT: A proper Arc<PageTable> would make ownership explicit.
    /// The current workaround is correct for QEMU single-address-space threads
    /// but needs review if the page table is ever freed while a thread is running.
    ///
    /// # Safety
    /// Must be called from process context with IRQs disabled.
    pub unsafe fn clone_thread(
        &mut self,
        parent_frame: *const ExceptionFrame,
        child_stack: u64,
        tls: u64,
    ) -> Result<Pid, ForkError> {
        let cpu_id = Self::current_cpu_id();
        let parent_pid = self.current_pids[cpu_id];

        // Collect parent data we need before borrowing `child`.
        let (parent_tgid, parent_nice, parent_signal_handlers, parent_fd_table_arc, parent_cwd,
             parent_cwd_path, parent_mmap_next_va, parent_mmap_regions) = {
            let parent = self.pool.get(parent_pid).ok_or(ForkError::InternalError)?;
            // clone_thread(): share the SAME FD table (POSIX thread semantics).
            let fd_table_arc = alloc::sync::Arc::clone(&parent.file_descriptor_table);
            let cwd = parent.cwd.clone();
            let cwd_path = parent.cwd_path.clone();
            let regions: alloc::vec::Vec<crate::process::MmapRegion> =
                parent.mmap_regions.iter().copied().collect();
            (
                parent.tgid,
                parent.nice,
                parent.signal_handlers,
                fd_table_arc,
                cwd,
                cwd_path,
                parent.mmap_next_va,
                regions,
            )
        };

        // Allocate a new process slot for the child thread.
        let child_pid = self.create_process(Some(parent_pid)).ok_or(ForkError::OutOfPids)?;

        // Configure the child thread.
        let child = self.pool.get_mut(child_pid).ok_or(ForkError::InternalError)?;

        // Threads share the parent's address space.  We leave `page_table = None`
        // because we cannot share the Box.  The scheduler's schedule_on_core()
        // path is patched to handle `is_thread == true` without clearing TTBR0.
        // See `schedule_on_core` for the corresponding change.
        child.is_thread = true;
        child.tgid = parent_tgid;
        child.nice = parent_nice;
        child.tls_base = tls;
        child.signal_handlers = parent_signal_handlers;
        // Threads share the parent's FD table (POSIX requirement).
        child.file_descriptor_table = parent_fd_table_arc;
        child.cwd = parent_cwd;
        child.cwd_path = parent_cwd_path;
        child.mmap_next_va = parent_mmap_next_va;
        child.mmap_regions = parent_mmap_regions;

        // Place the ExceptionFrame on the child's kernel stack.
        // Copy from parent's frame, then patch child-specific fields.
        let frame_destination = (child.kernel_stack.top as usize
            - core::mem::size_of::<ExceptionFrame>()) as *mut ExceptionFrame;
        core::ptr::copy_nonoverlapping(parent_frame, frame_destination, 1);

        // Child's clone() returns 0 (thread-create convention).
        (*frame_destination).x[0] = 0;

        // Set the child's user-space stack pointer to the caller-provided stack.
        (*frame_destination).sp = child_stack;

        // Configure the child's kernel-mode context.
        child.cpu_context.stack_pointer = frame_destination as u64;
        child.cpu_context.link_register = process_entry_trampoline_el0 as u64;

        // Enqueue the child on the least-loaded CPU.
        let child_pid = child.pid;
        let child_nice = child.nice;
        child.state = ProcessState::Ready;
        self.enqueue_load_balanced_with_nice(child_pid, child_nice);

        Ok(child_pid)
    }

    // -----------------------------------------------------------------------
    // exit() and wait()
    // -----------------------------------------------------------------------

    /// Mark the current process as a zombie and schedule away.
    ///
    /// Called from `sys_exit()`. The process releases its address space
    /// immediately; the slot is reclaimed when the parent calls `wait()`.
    ///
    /// Processes with no living parent (or whose parent never calls wait)
    /// are reparented to PID 1 (init) — TECHNICAL DEBT (Fase 5): for now
    /// we free them immediately.
    ///
    /// # Safety
    /// Must be called with IRQs disabled. Does not return.
    pub unsafe fn exit(&mut self, exit_code: i32) -> ! {
        let cpu_id = Self::current_cpu_id();
        let pid = self.current_pids[cpu_id];

        // Free the address space but keep the slot alive (zombie state).
        if let Some(process) = self.pool.get_mut(pid) {
            process.state = ProcessState::Zombie { exit_code };
            process.exit_code = exit_code;
            if process.is_thread {
                // Threads share the page table with their thread group leader.
                // Only the leader (is_thread == false) may free the physical frames.
                // A thread exit only needs to release its kernel stack, which
                // happens automatically when the Box<Process> is dropped on reap.
                // Do NOT set page_table = None (it is already None for threads;
                // see clone_thread) and do NOT free mmap regions that belong to
                // the shared address space.
            } else {
                // Non-thread (or thread group leader): free the address space.
                process.page_table = None;
                // Drop mmap regions (VAs are in the now-freed page table).
                process.mmap_regions.clear();
            }
        }

        // Reparent this process's children to init (bzinit) so their zombie
        // slots are reaped when bzinit calls wait(-1) in its supervisor loop.
        // Without this, orphans become permanently unreapable zombies.
        if let Some(init_pid) = self.init_pid {
            // Collect children first to avoid borrow conflict during mutation.
            let children: alloc::vec::Vec<Pid> = self.pool.slots.iter()
                .filter_map(|slot| {
                    if let ProcessSlot::Occupied(p) = slot {
                        if p.parent_pid == Some(pid) {
                            return Some(p.pid);
                        }
                    }
                    None
                })
                .collect();

            for child_pid in children {
                if let Some(child) = self.pool.get_mut(child_pid) {
                    child.parent_pid = Some(init_pid);
                }
                // If the child is already a zombie, wake init so it can reap it.
                if let Some(child) = self.pool.get(child_pid) {
                    if matches!(child.state, ProcessState::Zombie { .. }) {
                        self.wake_waiting_parent(child_pid);
                    }
                }
            }
        }

        // Wake any parent waiting on this PID.
        self.wake_waiting_parent(pid);

        // Switch to the next runnable process (never returns here).
        self.schedule_no_requeue();

        // If schedule_no_requeue returns (e.g., only idle is left), halt.
        loop {
            core::arch::asm!("wfe");
        }
    }

    /// Like `schedule()` but does NOT re-enqueue the current process.
    ///
    /// Used by `exit()` (zombie — must not re-enter the run queue) and by
    /// blocking syscalls (wait, read on empty pipe — caller set state to Blocked).
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn schedule_no_requeue(&mut self) {
        let cpu_id = Self::current_cpu_id();
        let from_pid = self.current_pids[cpu_id];

        let to_pid = self.run_queues[cpu_id]
            .pop()
            .or_else(|| self.steal_work(cpu_id))
            .unwrap_or(Pid::IDLE);

        if to_pid == from_pid {
            // Only idle is running. The idle process itself calls this when
            // it has nothing to do — it will re-enter via the timer IRQ.
            return;
        }

        let next_tls_base: u64 = if let Some(next) = self.pool.get_mut(to_pid) {
            next.state = ProcessState::Running;
            let tls = next.tls_base;
            if let Some(page_table) = &next.page_table {
                page_table.activate_el0();
            } else if next.is_thread {
                // Thread shares TTBR0 — do not clear it.
            } else {
                clear_ttbr0();
            }
            tls
        } else {
            0
        };

        self.current_pids[cpu_id] = to_pid;

        // Restore TLS for the incoming process/thread.
        // SAFETY: TPIDR_EL0 is a user-opaque register; EL1 write is always safe.
        core::arch::asm!(
            "msr tpidr_el0, {tls}",
            tls = in(reg) next_tls_base,
            options(nostack, nomem),
        );

        let from_ctx: *mut CpuContext = {
            match self.pool.get_mut(from_pid) {
                Some(process) => &mut process.cpu_context as *mut CpuContext,
                None => {
                    crate::drivers::uart::puts("[sched] schedule_no_requeue: from_pid not in pool idx=");
                    crate::drivers::uart::put_hex(from_pid.index as u64);
                    crate::drivers::uart::puts(" gen=");
                    crate::drivers::uart::put_hex(from_pid.generation as u64);
                    crate::drivers::uart::puts("\r\n");
                    panic!("schedule_no_requeue: from_pid not in pool");
                }
            }
        };
        let to_ctx: *const CpuContext = {
            match self.pool.get(to_pid) {
                Some(process) => &process.cpu_context as *const CpuContext,
                None => {
                    crate::drivers::uart::puts("[sched] schedule_no_requeue: to_pid not in pool idx=");
                    crate::drivers::uart::put_hex(to_pid.index as u64);
                    crate::drivers::uart::puts(" gen=");
                    crate::drivers::uart::put_hex(to_pid.generation as u64);
                    crate::drivers::uart::puts("\r\n");
                    panic!("schedule_no_requeue: to_pid not in pool");
                }
            }
        };

        scheduler_raw_lock_release();
        context_switch(from_ctx, to_ctx);
        scheduler_raw_lock_acquire();
    }

    /// Reap a zombie child of `parent_pid` and return its exit code.
    ///
    /// If `for_pid` is Some, only reap that specific child.
    /// If `for_pid` is None, reap any zombie child.
    ///
    /// Returns `None` if no matching zombie child exists.
    pub fn reap(&mut self, parent_pid: Pid, for_pid: Option<Pid>) -> Option<(Pid, i32)> {
        // Find a zombie child.
        let zombie_pid = {
            let mut found = None;
            for slot in self.pool.slots.iter() {
                if let ProcessSlot::Occupied(process) = slot {
                    let is_child = process.parent_pid == Some(parent_pid);
                    let is_zombie =
                        matches!(process.state, ProcessState::Zombie { .. });
                    let pid_matches = for_pid.map_or(true, |p| process.pid == p);
                    if is_child && is_zombie && pid_matches {
                        found = Some(process.pid);
                        break;
                    }
                }
            }
            found?
        };

        let exit_code = self.pool.free_slot(zombie_pid)?;
        Some((zombie_pid, exit_code))
    }

    // -----------------------------------------------------------------------
    // Blocking
    // -----------------------------------------------------------------------

    /// Block the current process and schedule away.
    ///
    /// Sets the current process state to `Blocked` so `schedule_no_requeue`
    /// will not re-enqueue it.  The caller is responsible for calling
    /// `unblock(pid)` when the waited event fires (e.g., from an IRQ handler
    /// or another process writing to a pipe).
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn block_current(&mut self) {
        let cpu_id = Self::current_cpu_id();
        let current_pid = self.current_pids[cpu_id];
        if let Some(process) = self.pool.get_mut(current_pid) {
            process.state = ProcessState::Blocked;
        }
        self.schedule_no_requeue();
    }

    /// Unblock a process that was previously blocked.
    ///
    /// Adds it back to the run queue.  Safe to call from IRQ context
    /// (no heap allocation, just a push onto the VecDeque).
    pub fn unblock(&mut self, pid: Pid) {
        if let Some(process) = self.pool.get_mut(pid) {
            let is_unblockable = matches!(
                process.state,
                ProcessState::Blocked | ProcessState::Waiting { .. }
            );
            if is_unblockable {
                let nice = process.nice;
                process.state = ProcessState::Ready;
                self.enqueue_load_balanced_with_nice(pid, nice);
            }
        }
    }

    /// Transition a process from `Sleeping` (or any non-Running state) to
    /// `Ready` and push it onto the run queue.
    ///
    /// Used by `sys_futex(FUTEX_WAKE)` to wake processes sleeping on a futex
    /// address without requiring them to be in `ProcessState::Blocked`.
    /// Unlike `unblock()`, this works for `Sleeping` state as well.
    pub fn futex_make_ready(&mut self, pid: Pid) {
        if let Some(process) = self.pool.get_mut(pid) {
            match process.state {
                ProcessState::Running | ProcessState::Zombie { .. } => {}
                _ => {
                    let nice = process.nice;
                    process.state = ProcessState::Ready;
                    self.enqueue_load_balanced_with_nice(pid, nice);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Signal delivery
    // -----------------------------------------------------------------------

    /// Send signal `signum` to the process identified by `pid`.
    ///
    /// Special handling:
    ///   - SIGSTOP (19): immediately move the target to `Stopped` regardless
    ///     of signal mask (SIGSTOP cannot be caught or ignored per POSIX).
    ///   - SIGCONT (18): resume a `Stopped` process by moving it to `Ready`.
    ///     Also delivered as a normal signal so the process can react.
    ///   - All others: if the target is Blocked, unblock it for prompt delivery.
    ///
    /// Returns `false` if the pid is not found or signum is invalid.
    ///
    /// Reference: POSIX.1-2017 "Signal Generation and Delivery".
    pub fn send_signal_to(&mut self, pid: Pid, signum: u8) -> bool {
        const SIGSTOP: u8 = 19;
        const SIGCONT: u8 = 18;

        if self.pool.get(pid).is_none() || signum == 0 || signum > 63 {
            return false;
        }

        match signum {
            SIGSTOP => {
                // SIGSTOP cannot be caught, ignored, or blocked.
                // Move the target to Stopped immediately.
                if let Some(process) = self.pool.get_mut(pid) {
                    match process.state {
                        ProcessState::Zombie { .. } | ProcessState::Stopped => {}
                        ProcessState::Ready => {
                            process.state = ProcessState::Stopped;
                            for queue in self.run_queues.iter_mut() {
                                queue.remove(pid);
                            }
                        }
                        _ => {
                            process.state = ProcessState::Stopped;
                        }
                    }
                }
                true
            }
            SIGCONT => {
                // Resume a stopped process; also record the signal.
                if let Some(process) = self.pool.get_mut(pid) {
                    process.send_signal(SIGCONT);
                    if process.state == ProcessState::Stopped {
                        let nice = process.nice;
                        process.state = ProcessState::Ready;
                        self.enqueue_load_balanced_with_nice(pid, nice);
                    }
                }
                true
            }
            _ => {
                let result = self
                    .pool
                    .get(pid)
                    .map(|process| process.send_signal(signum))
                    .unwrap_or(false);

                // Unblock a blocked/sleeping process so the signal is delivered quickly.
                if result {
                    if let Some(process) = self.pool.get_mut(pid) {
                        match process.state {
                            ProcessState::Blocked | ProcessState::Sleeping { .. } => {
                                let nice = process.nice;
                                process.state = ProcessState::Ready;
                                self.enqueue_load_balanced_with_nice(pid, nice);
                            }
                            _ => {}
                        }
                    }
                }

                result
            }
        }
    }

    /// Send `signum` to every process in process group `pgid`.
    ///
    /// Used by `kill(-pgid, sig)` and terminal signal delivery (Ctrl+C → SIGINT).
    /// Returns the count of processes that received the signal.
    pub fn send_signal_to_group(&mut self, pgid: u32, signum: u8) -> usize {
        // Collect PIDs first to avoid borrowing issues.
        let mut targets: alloc::vec::Vec<Pid> = alloc::vec::Vec::new();
        for slot in &self.pool.slots {
            if let ProcessSlot::Occupied(process) = slot {
                if process.pgid == pgid {
                    targets.push(process.pid);
                }
            }
        }
        let mut count = 0;
        for pid in targets {
            if self.send_signal_to(pid, signum) {
                count += 1;
            }
        }
        count
    }

    // -----------------------------------------------------------------------
    // mmap helpers — need both scheduler state and physical allocator
    // -----------------------------------------------------------------------

    /// Allocate `pages` anonymous pages and map them into the current process.
    ///
    /// Returns the base virtual address on success or `None` on failure.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn mmap_anonymous_for_current(
        &mut self,
        pages: usize,
        page_size: u64,
        phys: &mut crate::memory::PhysicalAllocator,
    ) -> Option<u64> {
        use crate::memory::VirtualAddress;
        use crate::memory::virtual_memory::PAGE_FLAGS_USER_DATA;

        let cpu_id = Self::current_cpu_id();
        let current_pid = self.current_pids[cpu_id];
        let process = self.pool.get_mut(current_pid)?;

        if process.mmap_regions.len() >= crate::process::MMAP_MAX_REGIONS {
            return None;
        }

        let base_va = process.mmap_next_va;
        let hhdm = phys.hhdm_offset();

        let page_table = process.page_table.as_mut()?;

        for page_index in 0..pages {
            let phys_page = phys.alloc()?;
            let phys_virt = phys_page.to_virtual(hhdm).as_ptr::<u8>();
            core::ptr::write_bytes(phys_virt, 0, page_size as usize);
            let va = VirtualAddress::new(base_va + page_index as u64 * page_size);
            page_table.map(va, phys_page, PAGE_FLAGS_USER_DATA, phys).ok()?;
        }

        let next_va = base_va + pages as u64 * page_size;
        let process = self.pool.get_mut(current_pid)?;
        process.mmap_next_va = next_va;
        process.mmap_regions.push(crate::process::MmapRegion {
            base: base_va,
            length: pages as u64 * page_size,
        });

        Some(base_va)
    }

    /// Map a range of pre-existing physical pages (e.g. the framebuffer) into
    /// the current process's address space.
    ///
    /// Unlike `mmap_anonymous_for_current`, this does NOT allocate physical
    /// memory — it maps pages that already exist (device memory allocated by
    /// the UEFI firmware).  The region is recorded in `mmap_regions` so that
    /// `munmap` can unmap it later without freeing the underlying pages.
    ///
    /// Returns the user virtual address of the first mapped page, or None on
    /// error.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.  `phys_base` must be a valid
    /// physical address range of at least `pages × page_size` bytes.
    pub unsafe fn map_physical_pages_for_current(
        &mut self,
        phys_base: u64,
        pages:     usize,
        page_size: u64,
        phys:      &mut crate::memory::PhysicalAllocator,
    ) -> Option<u64> {
        use crate::memory::{VirtualAddress, PhysicalAddress};
        use crate::memory::virtual_memory::PAGE_FLAGS_USER_DATA;

        let cpu_id = Self::current_cpu_id();
        let current_pid = self.current_pids[cpu_id];
        let process = self.pool.get_mut(current_pid)?;

        if process.mmap_regions.len() >= crate::process::MMAP_MAX_REGIONS {
            return None;
        }

        let base_va = process.mmap_next_va;
        let page_table = process.page_table.as_mut()?;

        for page_index in 0..pages {
            let phys_page = PhysicalAddress::new(phys_base + page_index as u64 * page_size);
            let va = VirtualAddress::new(base_va + page_index as u64 * page_size);
            page_table.map(va, phys_page, PAGE_FLAGS_USER_DATA, phys).ok()?;
        }

        let region_length = pages as u64 * page_size;
        let process = self.pool.get_mut(current_pid)?;
        process.mmap_next_va += region_length;
        process.mmap_regions.push(crate::process::MmapRegion {
            base:   base_va,
            length: region_length,
        });

        Some(base_va)
    }

    /// Unmap an anonymous region from the current process.
    ///
    /// Returns true if the region was found and unmapped.
    ///
    /// # Safety
    /// Must be called with IRQs disabled.
    pub unsafe fn munmap_for_current(
        &mut self,
        addr: u64,
        pages: usize,
        page_size: u64,
        phys: &mut crate::memory::PhysicalAllocator,
    ) -> bool {
        use crate::memory::VirtualAddress;

        let cpu_id = Self::current_cpu_id();
        let current_pid = self.current_pids[cpu_id];
        let process = match self.pool.get_mut(current_pid) {
            Some(p) => p,
            None => return false,
        };

        let before = process.mmap_regions.len();
        process.mmap_regions.retain(|r| r.base != addr);
        if process.mmap_regions.len() == before {
            return false; // region not found
        }

        if let Some(page_table) = process.page_table.as_mut() {
            page_table.unmap_range(VirtualAddress::new(addr), pages, page_size, phys);
        }
        true
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// PID of the currently executing process on the calling CPU.
    pub fn current_pid(&self) -> Pid {
        let cpu_id = Self::current_cpu_id();
        self.current_pids[cpu_id]
    }

    /// Return the PID indices of all living (non-idle) processes.
    ///
    /// Used by `ProcfsRootInode::readdir()` to enumerate `/proc` entries.
    /// Get a process by PID index (used by procfs).
    pub fn pool_get_by_index(&self, pid_index: usize) -> Option<&Process> {
        for slot in &self.pool.slots {
            if let ProcessSlot::Occupied(process) = slot {
                if process.pid.index as usize == pid_index {
                    return Some(process);
                }
            }
        }
        None
    }

    pub fn list_pids(&self) -> alloc::vec::Vec<u16> {
        let mut result = alloc::vec::Vec::new();
        for slot in &self.pool.slots {
            if let ProcessSlot::Occupied(process) = slot {
                if process.pid != Pid::IDLE {
                    result.push(process.pid.index);
                }
            }
        }
        result
    }

    /// Get a shared reference to the current process.
    pub fn current_process(&self) -> Option<&Process> {
        let cpu_id = Self::current_cpu_id();
        self.pool.get(self.current_pids[cpu_id])
    }

    /// Get a mutable reference to the current process.
    pub fn current_process_mut(&mut self) -> Option<&mut Process> {
        let cpu_id = Self::current_cpu_id();
        let current_pid = self.current_pids[cpu_id];
        self.pool.get_mut(current_pid)
    }

    /// Get a shared reference to any process by Pid.
    pub fn process(&self, pid: Pid) -> Option<&Process> {
        self.pool.get(pid)
    }

    /// Get a mutable reference to any process by Pid.
    pub fn process_mut(&mut self, pid: Pid) -> Option<&mut Process> {
        self.pool.get_mut(pid)
    }

    /// Number of alive (non-empty) processes in the pool.
    pub fn alive_process_count(&self) -> usize {
        self.pool.alive_count
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Wake a parent that is blocked in wait() waiting for `child_pid`.
    fn wake_waiting_parent(&mut self, child_pid: Pid) {
        // Find the child's parent PID first.
        let parent_pid = match self.pool.get(child_pid) {
            Some(process) => process.parent_pid,
            None => return,
        };
        let parent_pid = match parent_pid {
            Some(pid) => pid,
            None => return,
        };

        // Check if the parent is Waiting and interested in this child.
        let should_unblock = match self.pool.get(parent_pid) {
            Some(parent) => match parent.state {
                ProcessState::Waiting { for_pid: None } => true,
                ProcessState::Waiting { for_pid: Some(pid) } => pid == child_pid,
                _ => false,
            },
            None => false,
        };

        if should_unblock {
            self.unblock(parent_pid);
        }
    }
}

// ---------------------------------------------------------------------------
// PageTable helpers — activate user address space
// ---------------------------------------------------------------------------

use crate::memory::virtual_memory::PageTable;

impl PageTable {
    /// Activate this page table as the current user address space (TTBR0_EL1).
    ///
    /// # Safety
    /// Must be called with IRQs disabled. The page table must be fully mapped.
    pub fn activate_el0(&self) {
        unsafe {
            let ttbr0 = self.root_physical().as_u64();
            core::arch::asm!(
                "msr ttbr0_el1, {ttbr0}",
                "isb",
                "dsb sy",
                "tlbi vmalle1",
                "dsb sy",
                "isb",
                ttbr0 = in(reg) ttbr0,
                options(nostack),
            );
        }
    }
}

/// Clear TTBR0_EL1 (switch to no user address space — kernel-only process).
fn clear_ttbr0() {
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, xzr",
            "isb",
            "dsb sy",
            "tlbi vmalle1",
            "dsb sy",
            "isb",
            options(nostack),
        );
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkError {
    /// No free PID slot (PID_MAX reached).
    OutOfPids,
    /// Kernel heap could not satisfy the allocation.
    OutOfMemory,
    /// Internal scheduler inconsistency (should not happen in correct code).
    InternalError,
}

// ---------------------------------------------------------------------------
// Global scheduler instance
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Scheduler spinlock
//
// We use a raw AtomicBool spinlock rather than the SpinLock<T> wrapper because
// schedule() must release the lock *before* context_switch() and reacquire it
// after — a pattern that RAII guards cannot express without unsafe mem::forget.
//
// Protocol:
//   1. `with_scheduler` acquires the lock, calls the closure, releases it.
//   2. Inside `schedule()`, the lock is released before context_switch() and
//      reacquired after, so that other cores may schedule while this process
//      is suspended.
// ---------------------------------------------------------------------------

/// Global scheduler raw spinlock.
///
/// `false` = unlocked, `true` = locked.
/// Acquire/Release ordering on compare_exchange and store ensures correct
/// memory visibility on the weakly-ordered AArch64 memory model.
static SCHEDULER_RAW_LOCK: AtomicBool = AtomicBool::new(false);

/// Acquire the scheduler spinlock, spinning until it is available.
///
/// Uses `compare_exchange(false, true, Acquire, Relaxed)` which maps to a
/// load-acquire CAS on AArch64 (LDAXR/STXR or CASAL on ARMv8.1+).
/// Reference: ARM ARM DDI 0487 §B2.9 (acquire semantics).
pub(crate) fn scheduler_raw_lock_acquire() {
    while SCHEDULER_RAW_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        // Spin with a yield hint to reduce cache-line contention.
        core::hint::spin_loop();
    }
}

/// Release the scheduler spinlock.
///
/// `store(false, Release)` maps to a store-release (STLR) on AArch64,
/// ensuring all stores inside the critical section are visible to the next
/// lock holder before they observe the lock as free.
/// Reference: ARM ARM DDI 0487 §B2.9 (release semantics).
pub(crate) fn scheduler_raw_lock_release() {
    SCHEDULER_RAW_LOCK.store(false, Ordering::Release);
}

// We cannot use `const fn` to construct a `Scheduler` that contains
// `VecDeque` (which requires heap allocation and is therefore not const).
// Instead we use `MaybeUninit` and initialize the scheduler explicitly in
// `scheduler_init()` before any other scheduler function is called.
//
// SAFETY invariant: `SCHEDULER_CELL` is fully initialized by `scheduler_init()`
// before any call to `with_scheduler()`.  Calling `with_scheduler()` before
// `scheduler_init()` is undefined behaviour.
struct SyncSchedulerCell(UnsafeCell<core::mem::MaybeUninit<Scheduler>>);
unsafe impl Sync for SyncSchedulerCell {}

static SCHEDULER_CELL: SyncSchedulerCell =
    SyncSchedulerCell(UnsafeCell::new(core::mem::MaybeUninit::uninit()));

/// Initialise the global scheduler.
///
/// Must be called exactly once from `kernel_main()` before any use of the
/// scheduler.  Creates the idle process and readies the pool.
///
/// # Safety
/// Must be called from a single-threaded context with IRQs disabled.
pub unsafe fn scheduler_init() -> bool {
    // Write a freshly constructed Scheduler into the MaybeUninit slot.
    let scheduler = Scheduler::new();
    (*SCHEDULER_CELL.0.get()).write(scheduler);
    // Now run the idle-process initialization.
    (*SCHEDULER_CELL.0.get()).assume_init_mut().init().is_some()
}

/// Access the global scheduler while holding the scheduler spinlock.
///
/// The closure receives a `&mut Scheduler`.  The spinlock is acquired before
/// calling the closure and released after it returns.
///
/// # Safety
/// Must be called after `scheduler_init()`.
/// IRQs should be disabled by the caller if called from a context where a
/// timer IRQ could re-enter the scheduler (e.g. the idle loop).
#[inline(always)]
pub unsafe fn with_scheduler<F, R>(function: F) -> R
where
    F: FnOnce(&mut Scheduler) -> R,
{
    scheduler_raw_lock_acquire();
    // SAFETY: SCHEDULER_CELL is initialized by scheduler_init() before any
    // call to with_scheduler().
    let result = function((*SCHEDULER_CELL.0.get()).assume_init_mut());
    scheduler_raw_lock_release();
    result
}

/// Schedule the next process from the idle loop (BSP or AP).
///
/// Acquires the scheduler lock, runs `schedule()` (which may context-switch
/// to another process), and releases the lock.  Safe to call from any core.
///
/// # Safety
/// Must be called after `scheduler_init()`.
pub unsafe fn schedule_next() {
    scheduler_raw_lock_acquire();
    // SAFETY: SCHEDULER_CELL is initialized; lock is held.
    let cpu_id = {
        // TPIDR_EL1 may be zero very early in boot (before per_cpu_init).
        // current_cpu_id() defensively clamps to [0, MAX_CPUS).
        let raw_id = smp::current_cpu().cpu_id as usize;
        raw_id.min(MAX_CPUS - 1)
    };
    (*SCHEDULER_CELL.0.get())
        .assume_init_mut()
        .schedule_on_core(cpu_id);
    // schedule_on_core() releases and reacquires the lock around context_switch().
    // On return here the lock is held again; release it now.
    scheduler_raw_lock_release();
}
