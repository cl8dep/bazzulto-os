// smp/mod.rs — Symmetric Multi-Processing support structures.
//
// Per-CPU data is accessed via TPIDR_EL1 (EL1 thread ID register).
// Each CPU core writes its own `PerCpuData` block address into TPIDR_EL1
// during `per_cpu_init()`.  All subsequent accesses use `current_cpu()`.
//
// Reference: ARM ARM DDI 0487 §D13.2.113 (TPIDR_EL1).

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::process::Pid;

/// Maximum number of CPU cores supported.
///
/// QEMU `virt` supports up to 8 cores; 4 is a conservative starting point
/// matching common dual/quad-core embedded SoCs.
pub const MAX_CPUS: usize = 4;

/// Cache-line size on ARM Cortex-A series (64 bytes).
///
/// Each `PerCpuData` block is padded to this size so that per-CPU fields
/// from different cores do not share a cache line (false sharing).
///
/// Reference: ARM Cortex-A53/A55/A72/A76 MPCore Technical Reference Manuals,
///            §L1 data cache line size = 64 bytes.
const CACHE_LINE_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// PerCpuData
// ---------------------------------------------------------------------------

/// Per-CPU state block, one instance per logical CPU core.
///
/// Placed at a cache-line boundary so that accesses from one core do not
/// cause false-sharing invalidations on neighbouring cores' copies.
///
/// # Memory layout (repr C, verified by compile-time assert below)
///
/// | Offset | Field        | Size |
/// |--------|------------- |------|
/// | 0      | cpu_id       |  4   |
/// | 4      | current_pid  |  3   |
/// | 7      | (padding)    |  1   |
/// | 8      | kernel_sp    |  8   |
/// | 16     | _padding     | 48   |
/// | 64     | (end)        |      |
#[repr(C)]
pub struct PerCpuData {
    /// Zero-based index of this CPU core.
    pub cpu_id: u32,
    /// PID of the process/thread currently executing on this CPU.
    pub current_pid: Pid,
    /// EL1 stack pointer used when entering the kernel from EL0.
    ///
    /// Set during per-CPU initialisation and restored on each exception entry
    /// (future work: SP_EL1 is set per-process on context switch).
    pub kernel_sp: u64,
    /// Explicit padding to fill the cache line.
    ///
    /// The size must satisfy:
    ///   `size_of::<PerCpuData>() == CACHE_LINE_SIZE`
    /// The compile-time assert below enforces this.
    _padding: [u8; CACHE_LINE_SIZE - 4 - 3 - 1 - 8],
}

// Compile-time check: PerCpuData must be exactly one cache line.
const _: () = {
    if core::mem::size_of::<PerCpuData>() != CACHE_LINE_SIZE {
        panic!("PerCpuData size does not equal CACHE_LINE_SIZE — adjust _padding");
    }
};

impl PerCpuData {
    /// Construct a zeroed `PerCpuData` for CPU `cpu_id`.
    ///
    /// `const fn` so it can be used to initialise the `PER_CPU` static array.
    pub const fn zeroed(cpu_id: u32) -> Self {
        Self {
            cpu_id,
            current_pid: Pid::IDLE,
            kernel_sp: 0,
            _padding: [0u8; CACHE_LINE_SIZE - 4 - 3 - 1 - 8],
        }
    }
}

// ---------------------------------------------------------------------------
// Static per-CPU array
// ---------------------------------------------------------------------------

/// One `PerCpuData` block per potential CPU core.
///
/// Indexed by CPU ID (0-based).  Access must go through `per_cpu_init()` /
/// `current_cpu()` to ensure TPIDR_EL1 is valid before use.
///
/// # Safety
/// Mutable only during `per_cpu_init()` on each CPU's boot path.
/// After initialisation, each CPU accesses only its own slot, so no
/// synchronisation between CPUs is needed for the TPIDR_EL1-accessed fields.
static mut PER_CPU: [PerCpuData; MAX_CPUS] = [
    PerCpuData::zeroed(0),
    PerCpuData::zeroed(1),
    PerCpuData::zeroed(2),
    PerCpuData::zeroed(3),
];

// ---------------------------------------------------------------------------
// Initialisation and access
// ---------------------------------------------------------------------------

/// Initialise per-CPU data for CPU `cpu_id` and install TPIDR_EL1.
///
/// Writes the address of `PER_CPU[cpu_id]` into the TPIDR_EL1 register so
/// that `current_cpu()` can retrieve it without an index lookup.
///
/// # Safety
/// Must be called exactly once per CPU during SMP bring-up, on the CPU being
/// initialised, before any call to `current_cpu()` on that CPU.
/// `cpu_id` must be less than `MAX_CPUS`.
pub unsafe fn per_cpu_init(cpu_id: u32) {
    debug_assert!(
        (cpu_id as usize) < MAX_CPUS,
        "per_cpu_init: cpu_id {} >= MAX_CPUS {}",
        cpu_id,
        MAX_CPUS
    );

    let slot_ptr: *mut PerCpuData = &raw mut PER_CPU[cpu_id as usize];

    // Write the pointer into TPIDR_EL1.
    // TPIDR_EL1 is a 64-bit general-purpose register that EL1 software may
    // use freely.  We repurpose it as a per-CPU base pointer.
    // Reference: ARM ARM DDI 0487 §D13.2.113.
    core::arch::asm!(
        "msr tpidr_el1, {ptr}",
        ptr = in(reg) slot_ptr as u64,
        options(nostack, nomem),
    );
}

/// Return a mutable reference to the current CPU's `PerCpuData`.
///
/// Reads TPIDR_EL1 and casts to `&mut PerCpuData`.
///
/// # Safety
/// `per_cpu_init` must have been called for this CPU before this function is
/// used.  If TPIDR_EL1 is zero or holds a stale value the returned reference
/// will be invalid.
pub unsafe fn current_cpu() -> &'static mut PerCpuData {
    let ptr: u64;
    core::arch::asm!(
        "mrs {ptr}, tpidr_el1",
        ptr = out(reg) ptr,
        options(nostack, nomem),
    );
    debug_assert!(ptr != 0, "current_cpu: TPIDR_EL1 is zero — per_cpu_init not called?");
    &mut *(ptr as *mut PerCpuData)
}

// ---------------------------------------------------------------------------
// AP (Application Processor) bringup
// ---------------------------------------------------------------------------

/// Number of pages (4 KiB each) for each AP kernel stack: 64 KiB total.
const AP_STACK_SIZE_PAGES: usize = 16;

/// Size in bytes of one AP kernel stack.
const AP_STACK_SIZE_BYTES: usize = AP_STACK_SIZE_PAGES * 4096;

/// Wrapper type for AP stack storage.
///
/// `repr(align(16))` ensures the stack is 16-byte aligned, satisfying the
/// AArch64 AAPCS64 stack alignment requirement at function call boundaries.
/// Reference: ARM IHI 0055 §6.2.3 (stack must be 16-byte aligned).
#[repr(align(16))]
struct ApplicationProcessorStack([u8; AP_STACK_SIZE_BYTES]);

/// Statically allocated kernel stacks for AP cores (one per non-BSP core).
///
/// Indexed by `cpu_id - 1` (BSP uses the stack set up in start.S).
///
/// # Safety
/// Each AP writes its SP once during bringup and owns its stack exclusively
/// thereafter.  No synchronisation is required between cores for stack access.
static mut AP_STACKS: [ApplicationProcessorStack; MAX_CPUS - 1] = [
    ApplicationProcessorStack([0; AP_STACK_SIZE_BYTES]),
    ApplicationProcessorStack([0; AP_STACK_SIZE_BYTES]),
    ApplicationProcessorStack([0; AP_STACK_SIZE_BYTES]),
];

/// Return the top-of-stack virtual address for AP core `cpu_id`.
///
/// The stack grows downward; `ap_stack_top` returns the address one byte
/// past the end of the allocated region.
///
/// # Panics
/// Panics in debug builds if `cpu_id == 0` (BSP) or `>= MAX_CPUS`.
pub fn ap_stack_top(cpu_id: usize) -> u64 {
    debug_assert!(cpu_id >= 1 && cpu_id < MAX_CPUS,
        "ap_stack_top: cpu_id {} out of range [1, {})", cpu_id, MAX_CPUS);
    // SAFETY: cpu_id is validated above; indexing into AP_STACKS is in-bounds.
    let stack_base = unsafe { AP_STACKS[cpu_id - 1].0.as_ptr() as u64 };
    stack_base + AP_STACK_SIZE_BYTES as u64
}

/// Count of AP cores that have completed their initialisation sequence.
///
/// Incremented with `Release` ordering by each AP at the end of `ap_entry`.
/// The BSP waits on this counter (with `Acquire` loads) before proceeding.
pub static AP_ONLINE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Entry point for all Application Processor (AP) cores.
///
/// Called by Limine's SMP trampoline.  On entry the CPU is at EL1 with the
/// MMU enabled and HHDM mapped (identical initial state to the BSP).
/// IRQs are masked (DAIF.I = 1) on entry.
///
/// The cpu_id for this core was stored in `SmpCpuInfo::extra_argument` by the
/// BSP before writing `goto_address`.
///
/// # Safety
/// Called exactly once per AP, from Limine's trampoline.
#[no_mangle]
pub unsafe extern "C" fn ap_entry(info: *const crate::limine::SmpCpuInfo) -> ! {
    // Retrieve the cpu_id stored by the BSP in extra_argument.
    // SAFETY: `info` is a valid pointer to this core's SmpCpuInfo; Limine
    // guarantees it is live and mapped for the duration of boot.
    let cpu_id = unsafe { (*info).extra_argument as u32 };

    // Install this AP's kernel stack.
    //
    // Limine's trampoline uses a temporary stack; we switch to our own
    // statically allocated stack immediately so the rest of init is on a
    // known-good region.
    let stack_top = ap_stack_top(cpu_id as usize);
    unsafe {
        core::arch::asm!(
            "mov sp, {sp}",
            sp = in(reg) stack_top,
            options(nostack, nomem),
        );
    }

    // Install per-CPU data and write TPIDR_EL1 for this core.
    // SAFETY: called exactly once for this cpu_id; cpu_id < MAX_CPUS.
    unsafe { per_cpu_init(cpu_id) };

    // Install the exception vector table on this core.
    // SAFETY: called from EL1 with a valid stack.
    unsafe { crate::arch::arm64::exceptions::exceptions_init() };

    // Initialise this core's GIC CPU interface.
    // SAFETY: BSP has already run platform_init(); the GIC distributor is
    // globally configured.  This call only configures the per-CPU GICC.
    unsafe { crate::platform::qemu_virt::ap_gic_cpu_interface_init() };

    // Arm the EL1 physical timer on this core.
    // The Generic Timer is per-CPU; each core must arm it independently.
    // Reference: ARM ARM DDI 0487 §D11.2 (CNTP_CTL_EL0 / CNTP_CVAL_EL0 are
    // banked per PE).
    // SAFETY: called from EL1; CNTFRQ_EL0 is valid after firmware init.
    unsafe { crate::platform::qemu_virt::ap_timer_arm_this_core() };

    // Signal that this AP has completed initialisation.
    AP_ONLINE_COUNT.fetch_add(1, Ordering::Release);

    crate::drivers::uart::puts("SMP: AP core online\r\n");

    // Unmask IRQs so the timer can preempt this core.
    // Reference: ARM ARM DDI 0487 §C5.2.4 DAIF.I bit.
    unsafe {
        core::arch::asm!("msr daifclr, #2", options(nostack, nomem));
    }

    // Enter the scheduler loop: same as the BSP's idle loop.
    loop {
        unsafe {
            crate::scheduler::schedule_next();
            core::arch::asm!("wfe", options(nomem, nostack));
        }
    }
}
