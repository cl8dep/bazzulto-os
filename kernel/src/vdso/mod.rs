//! vDSO — Virtual Dynamic Shared Object for Bazzulto OS.
//!
//! The kernel generates a 4 KiB read-only code page containing one stub per
//! syscall slot.  Each stub is exactly 16 bytes (4 words):
//!
//!   svc #N      (4 bytes) — invoke syscall N
//!   ret         (4 bytes) — return to caller
//!   nop         (4 bytes) — padding
//!   nop         (4 bytes) — padding
//!
//! **Fast clock_gettime (slot 19)**
//!
//! Slot 19 is replaced by a direct branch to a larger implementation appended
//! at offset 0x480 in the same code page (the NOP-filled region after the last
//! slot).  The fast path reads `CNTPCT_EL0` and `CNTFRQ_EL0` directly from
//! EL0 (no privilege trap needed), and reads `boot_rtc_seconds` from the vDSO
//! data page at VA 0x3000 for `CLOCK_REALTIME`.
//!
//! Unknown clock IDs fall back to `svc #19` (kernel trap).
//!
//! # AArch64 encoding notes
//!
//! `svc #N`  = `0xD4000001 | (N << 5)`        (bits [20:5] = imm16)
//! `ret`     = `0xD65F03C0`
//! `nop`     = `0xD503201F`
//! `B imm26` = `0x14000000 | imm26`           (unconditional branch)
//! `B.cond`  = `0x54000000 | (imm19 << 5) | cond`
//!
//! Reference: ARM DDI 0487 — C6.2.317 (SVC), C6.2.218 (RET), C6.2.30 (B),
//!            C6.2.32 (B.cond), C6.2.190 (MRS).

use crate::memory::address::{PhysicalAddress, VirtualAddress};
use crate::memory::virtual_memory::{PageTable, MapError, PAGE_FLAGS_USER_CODE, PAGE_FLAGS_USER_DATA_READ_ONLY};
use crate::memory::{PhysicalAllocator, kernel_va_to_pa};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Virtual address at which the vDSO code page is mapped in every user process.
///
/// This value is permanent — changing it breaks all compiled userspace binaries.
pub const VDSO_BASE_VA: usize = 0x1000;

/// Virtual address of the read-only vDSO data page.
///
/// Contains `VdsoData` (boot_rtc_seconds at offset 0) shared with userspace.
/// The fast clock_gettime implementation reads `boot_rtc_seconds` from here
/// for `CLOCK_REALTIME`.
pub const VDSO_DATA_VA: usize = 0x3000;

/// Number of syscall slots in the vDSO.
///
/// Must be ≥ (highest syscall number + 1). Increase when adding syscalls.
/// Current highest: GETMOUNTS = 114 → need at least 115 slots.
pub const VDSO_SLOT_COUNT: usize = 115;

/// Bytes per slot: 4 instructions × 4 bytes.
pub const VDSO_SLOT_SIZE: usize = 16;

/// Word index within the code page where the fast clock_gettime implementation
/// begins.  Must be > VDSO_SLOT_COUNT * 4 (last slot word = 114*4+3 = 459).
/// Using 464 (word offset 0x730 / byte offset 0x1CC0 from page start).
const FAST_CLOCK_GETTIME_WORD_OFFSET: usize = 464;

const PAGE_SIZE: usize = 4096;

// AArch64 instruction words.
const SVC_BASE: u32 = 0xD4000001; // svc #0 — OR in (N << 5) for svc #N
const RET:      u32 = 0xD65F03C0;
const NOP:      u32 = 0xD503201F;

// Slot number for clock_gettime.
const SYSCALL_CLOCK_GETTIME: usize = 19;

// ---------------------------------------------------------------------------
// AArch64 instruction encodings for fast clock_gettime
//
// Reference: ARM DDI 0487, instruction descriptions.
// ---------------------------------------------------------------------------

// mrs x2, CNTPCT_EL0  (physical counter, readable from EL0)
// System register encoding: op0=3, op1=3, CRn=14, CRm=0, op2=2 → S3_3_c14_c0_1 = CNTPCT_EL0
// MRS encoding: 1101 0101 0011 op1 CRn CRm op2 Rt
// CNTPCT_EL0: op0=3(0b11), op1=3, CRn=14(0b1110), CRm=0, op2=1
// = 0xD53BE042
const MRS_X2_CNTPCT_EL0: u32 = 0xD53BE042;

// mrs x3, CNTFRQ_EL0  (counter frequency, readable from EL0)
// CNTFRQ_EL0: op0=3, op1=3, CRn=14, CRm=0, op2=0 → 0xD53BE003
const MRS_X3_CNTFRQ_EL0: u32 = 0xD53BE003;

// udiv x4, x2, x3  — seconds = cntpct / freq
// UDIV Xd, Xn, Xm: 0x9AC00000 | (Xm<<16) | (Xn<<5) | Xd
// = 0x9AC00000 | (3<<16) | (2<<5) | 4 = 0x9AC30844
const UDIV_X4_X2_X3: u32 = 0x9AC30844;

// msub x5, x4, x3, x2  — remainder = x2 - x4*x3
// MSUB Xd, Xn, Xm, Xa: 0x9B000000 | (Xm<<16) | (1<<15) | (Xa<<10) | (Xn<<5) | Xd
// Xd=5, Xn=4, Xm=3, Xa=2
// = 0x9B000000 | (3<<16) | 0x8000 | (2<<10) | (4<<5) | 5
// = 0x9B000000 | 0x30000 | 0x8000 | 0x800 | 0x80 | 5 = 0x9B038885
const MSUB_X5_X4_X3_X2: u32 = 0x9B038885;

// movz x7, #0xCA00  — low 16 bits of 1_000_000_000 (= 0x3B9ACA00)
// MOVZ Xd, #imm16, lsl #0: 0xD2800000 | (imm16<<5) | Xd
// imm16=0xCA00, Xd=7: 0xD2800000 | (0xCA00<<5) | 7 = 0xD2800000 | 0x1940000 | 7 = 0xD2994007
const MOVZ_X7_LO: u32 = 0xD2994007;

// movk x7, #0x3B9A, lsl #16  — high 16 bits of 1_000_000_000
// MOVK Xd, #imm16, lsl #16: 0xF2A00000 | (imm16<<5) | Xd
// imm16=0x3B9A, Xd=7: 0xF2A00000 | (0x3B9A<<5) | 7 = 0xF2A00000 | 0x77400 | 7 = 0xF2A77407
const MOVK_X7_HI: u32 = 0xF2A77407;

// mul x5, x5, x7  — nanoseconds of remainder
// MUL = MADD Xd, Xn, Xm, XZR: 0x9B000000 | (Xm<<16) | (0<<15) | (31<<10) | (Xn<<5) | Xd
// Xd=5, Xn=5, Xm=7: 0x9B000000 | (7<<16) | 0 | (31<<10) | (5<<5) | 5
// = 0x9B000000 | 0x70000 | 0x7C00 | 0xA0 | 5 = 0x9B077CA5
const MUL_X5_X5_X7: u32 = 0x9B077CA5;

// udiv x5, x5, x3  — nanoseconds = (remainder * 1e9) / freq
// UDIV Xd, Xn, Xm: 0x9AC00800 | (Xm<<16) | (Xn<<5) | Xd
// Bits [12:10] = 0b010 (UDIV opcode) must be ORed into the base as 0x800.
// Xd=5, Xn=5, Xm=3: 0x9AC00800 | (3<<16) | (5<<5) | 5 = 0x9AC308A5
// Reference: ARM DDI 0487, Data Processing (2-source), UDIV encoding.
const UDIV_X5_X5_X3: u32 = 0x9AC308A5;

// add x4, x4, x6  — add epoch offset (0 for MONOTONIC, boot_rtc_seconds for REALTIME)
// ADD (shifted, 64-bit): 0x8B000000 | (Xm<<16) | (Xn<<5) | Xd
// Xd=4, Xn=4, Xm=6: 0x8B000000 | (6<<16) | (4<<5) | 4 = 0x8B060084
const ADD_X4_X4_X6: u32 = 0x8B060084;

// stp x4, x5, [x1]  — store tv_sec and tv_nsec to caller's timespec
// STP Xt1, Xt2, [Xn, #0]: 0xA9000000 | (imm7<<15) | (Xt2<<10) | (Xn<<5) | Xt1
// Xt1=4, Xt2=5, Xn=1, imm7=0: 0xA9000000 | 0 | (5<<10) | (1<<5) | 4 = 0xA9001424
const STP_X4_X5_X1: u32 = 0xA9001424;

// mov x6, #0  — monotonic epoch offset = 0
// MOV Xd, #0 = MOVZ Xd, #0: 0xD2800000 | 6 = 0xD2800006
const MOV_X6_ZERO: u32 = 0xD2800006;

// mov x9, #0x3000  — VDSO data page VA
// MOVZ Xd, #imm16: 0xD2800000 | (imm16<<5) | Xd
// imm16=0x3000, Xd=9: 0xD2800000 | (0x3000<<5) | 9 = 0xD2800000 | 0x60000 | 9 = 0xD2860009
const MOV_X9_VDSO_DATA: u32 = 0xD2860009;

// ldr x6, [x9, #0]  — load boot_rtc_seconds from data page
// LDR Xt, [Xn, #imm12]: 0xF9400000 | (imm12<<10) | (Xn<<5) | Xt
// Xt=6, Xn=9, imm12=0: 0xF9400000 | 0 | (9<<5) | 6 = 0xF9400126
const LDR_X6_X9_0: u32 = 0xF9400126;

// mov x0, #0  — return success
const MOV_X0_ZERO: u32 = 0xD2800000;

// svc #19  — fallback trap for unknown clock IDs
// svc #N = 0xD4000001 | (N << 5); N=19: 0xD4000001 | (19<<5) = 0xD4000261
const SVC_CLOCK_GETTIME: u32 = 0xD4000261;

// cmp x0, #1  — SUBS xzr, x0, #1
// SUBS Xd, Xn, #imm: 0xF1000000 | (imm12<<10) | (Xn<<5) | Xd
// Xd=XZR=31, Xn=x0=0, imm=1: 0xF1000000 | (1<<10) | (0<<5) | 31 = 0xF100041F
const CMP_X0_1: u32 = 0xF100041F;

// cmp x0, #0  — SUBS xzr, x0, #0
// SUBS Xd, Xn, #0: 0xF1000000 | (0<<10) | (0<<5) | 31 = 0xF100001F
const CMP_X0_0: u32 = 0xF100001F;

// ---------------------------------------------------------------------------
// Static vDSO code page
// ---------------------------------------------------------------------------

/// The vDSO code page contents — generated at compile time.
///
/// Laid out as `PAGE_SIZE / 4` u32 words. Slot N occupies words [N*4 .. N*4+4].
/// Words beyond `VDSO_SLOT_COUNT * 4` are all NOP, except slot 19 (replaced
/// with B → fast_clock_gettime) and the fast implementation itself at word 464.
#[repr(C, align(4096))]
struct VdsoPage([u32; PAGE_SIZE / 4]);

const fn generate_vdso_page() -> VdsoPage {
    let mut words = [NOP; PAGE_SIZE / 4];

    // Generate stubs for all slots.
    let mut slot = 0usize;
    while slot < VDSO_SLOT_COUNT {
        let base = slot * 4;
        words[base]     = SVC_BASE | ((slot as u32) << 5); // svc #slot
        words[base + 1] = RET;
        words[base + 2] = NOP;
        words[base + 3] = NOP;
        slot += 1;
    }

    // Replace slot 19 (clock_gettime) with a direct branch to the fast impl.
    //
    // Slot 19 base word index: 19 * 4 = 76.
    // Fast impl base word index: FAST_CLOCK_GETTIME_WORD_OFFSET = 416.
    // Branch imm26 = 416 - 76 = 340.
    // B encoding: 0x14000000 | imm26.
    {
        let slot_base = SYSCALL_CLOCK_GETTIME * 4; // 76
        let target = FAST_CLOCK_GETTIME_WORD_OFFSET; // 288
        let imm26 = (target - slot_base) as u32; // 212
        words[slot_base]     = 0x14000000 | imm26; // B #fast_clock_gettime
        words[slot_base + 1] = NOP;
        words[slot_base + 2] = NOP;
        words[slot_base + 3] = NOP;
    }

    // ---------------------------------------------------------------------------
    // Fast clock_gettime implementation at word 416 (byte offset 0x680).
    //
    // Inputs:  x0 = clk_id (0 = CLOCK_REALTIME, 1 = CLOCK_MONOTONIC)
    //          x1 = *mut timespec (caller-allocated, two u64 fields: tv_sec, tv_nsec)
    // Outputs: x0 = 0 on success; falls back to svc #19 for unknown clocks.
    //
    // Layout (word index relative to FAST_CLOCK_GETTIME_WORD_OFFSET = 416):
    //
    //   local  abs   label
    //     0    416   entry: cmp x0, #1
    //     1    417          b.eq .monotonic   (abs 423, imm19 = 6)
    //     2    418          cmp x0, #0
    //     3    419          b.ne .fallback    (abs 436, imm19 = 17)
    //   .realtime (local 4, abs 420):
    //     4    420          mov x9, #0x3000   (VDSO_DATA_VA)
    //     5    421          ldr x6, [x9, #0]  (boot_rtc_seconds)
    //     6    422          b .compute        (abs 424, imm26 = 2)
    //   .monotonic (local 7, abs 423):
    //     7    423          mov x6, #0        (no epoch offset)
    //   .compute (local 8, abs 424):
    //     8    424          mrs x2, CNTPCT_EL0
    //     9    425          mrs x3, CNTFRQ_EL0
    //    10    426          udiv x4, x2, x3   (seconds)
    //    11    427          msub x5, x4, x3, x2  (remainder cycles)
    //    12    428          movz x7, #0xCA00  (low 16 bits of 1e9)
    //    13    429          movk x7, #0x3B9A, lsl 16  (high 16 bits of 1e9)
    //    14    430          mul x5, x5, x7    (remainder * 1e9)
    //    15    431          udiv x5, x5, x3   (nanoseconds)
    //    16    432          add x4, x4, x6    (add epoch offset)
    //    17    433          stp x4, x5, [x1]  (store tv_sec, tv_nsec)
    //    18    434          mov x0, #0        (return 0)
    //    19    435          ret
    //   .fallback (local 20, abs 436):
    //    20    436          svc #19           (unknown clock → kernel)
    //    21    437          ret
    // ---------------------------------------------------------------------------

    let b = FAST_CLOCK_GETTIME_WORD_OFFSET; // 416 (base absolute word index)

    // B.cond encoding: 0x54000000 | (imm19 << 5) | cond
    // cond EQ = 0, cond NE = 1
    // b.eq .monotonic: abs 289 → abs 295, imm19 = 6
    let b_eq_monotonic: u32 = 0x54000000 | (6u32 << 5) | 0; // 0x540000C0
    // b.ne .fallback: abs 291 → abs 308, imm19 = 17
    let b_ne_fallback: u32  = 0x54000000 | (17u32 << 5) | 1; // 0x54000221
    // b .compute: abs 294 → abs 296, imm26 = 2
    let b_compute: u32      = 0x14000000 | 2u32; // 0x14000002

    words[b + 0]  = CMP_X0_1;
    words[b + 1]  = b_eq_monotonic;
    words[b + 2]  = CMP_X0_0;
    words[b + 3]  = b_ne_fallback;
    // .realtime
    words[b + 4]  = MOV_X9_VDSO_DATA;
    words[b + 5]  = LDR_X6_X9_0;
    words[b + 6]  = b_compute;
    // .monotonic
    words[b + 7]  = MOV_X6_ZERO;
    // .compute
    words[b + 8]  = MRS_X2_CNTPCT_EL0;
    words[b + 9]  = MRS_X3_CNTFRQ_EL0;
    words[b + 10] = UDIV_X4_X2_X3;
    words[b + 11] = MSUB_X5_X4_X3_X2;
    words[b + 12] = MOVZ_X7_LO;
    words[b + 13] = MOVK_X7_HI;
    words[b + 14] = MUL_X5_X5_X7;
    words[b + 15] = UDIV_X5_X5_X3;
    words[b + 16] = ADD_X4_X4_X6;
    words[b + 17] = STP_X4_X5_X1;
    words[b + 18] = MOV_X0_ZERO;
    words[b + 19] = RET;
    // .fallback
    words[b + 20] = SVC_CLOCK_GETTIME;
    words[b + 21] = RET;

    VdsoPage(words)
}

/// Statically allocated, 4 KiB-aligned vDSO code page.
static VDSO_PAGE: VdsoPage = generate_vdso_page();

// ---------------------------------------------------------------------------
// vDSO data page
// ---------------------------------------------------------------------------

/// Data shared between the kernel and userspace via the vDSO data page.
///
/// Mapped read-only into every user process at `VDSO_DATA_VA` (0x3000).
/// The fast clock_gettime implementation reads `boot_rtc_seconds` to compute
/// `CLOCK_REALTIME` without a kernel trap.
///
/// Layout is stable: `boot_rtc_seconds` is always at offset 0.
/// Reference: Linux `struct vdso_data` (arch/arm64/include/asm/vdso/vdso_data.h).
#[repr(C, align(4096))]
pub struct VdsoData {
    /// Unix epoch seconds recorded at kernel boot (from the PL031 RTC).
    ///
    /// Written once by `vdso_set_boot_rtc_seconds()` during platform init.
    /// Never changes after that — userspace reads it as a constant offset.
    pub boot_rtc_seconds: u64,
    _padding: [u64; 511],
}

impl VdsoData {
    const fn zeroed() -> Self {
        Self { boot_rtc_seconds: 0, _padding: [0u64; 511] }
    }
}

/// Statically allocated, 4 KiB-aligned vDSO data page.
///
/// # Safety
/// Written exactly once from `vdso_set_boot_rtc_seconds()` before any user
/// process is created. All subsequent accesses are read-only.
static mut VDSO_DATA_PAGE: VdsoData = VdsoData::zeroed();

/// Record the boot-time RTC value into the vDSO data page.
///
/// Must be called during platform initialisation, before the first user
/// process is created. `seconds` is the Unix epoch second count at boot.
///
/// # Safety
/// Must be called exactly once, before any user process exists.
pub unsafe fn vdso_set_boot_rtc_seconds(seconds: u64) {
    VDSO_DATA_PAGE.boot_rtc_seconds = seconds;
}

// ---------------------------------------------------------------------------
// Physical address helpers
// ---------------------------------------------------------------------------

/// Return the physical address of the vDSO code page.
///
/// The page lives in kernel `.rodata` (VDSO_PAGE) or `.bss` (VDSO_DATA_PAGE),
/// both mapped at `kernel_virt_base` (not at HHDM offset).
/// Physical address = kernel_phys_base + (virt − kernel_virt_base).
///
/// Must be called after `memory_init()` so the kernel base globals are set.
pub fn vdso_physical_address() -> PhysicalAddress {
    let virt = &VDSO_PAGE as *const VdsoPage as u64;
    PhysicalAddress::new(kernel_va_to_pa(virt))
}

/// Return the physical address of the vDSO data page.
pub fn vdso_data_physical_address() -> PhysicalAddress {
    let virt = unsafe { &VDSO_DATA_PAGE as *const VdsoData as u64 };
    PhysicalAddress::new(kernel_va_to_pa(virt))
}

// ---------------------------------------------------------------------------
// Map into a user process
// ---------------------------------------------------------------------------

/// Map the vDSO code page read-only into `page_table` at `VDSO_BASE_VA`.
///
/// Called from `load_elf()` and `fork()`. The physical page is shared
/// (read-only) across all processes — no per-process copy is made.
///
/// # Safety
/// Must be called with IRQs disabled. `allocator` must be the live physical
/// allocator (needed to allocate page-table walk pages).
pub unsafe fn vdso_map_into_process(
    page_table: &mut PageTable,
    allocator: &mut PhysicalAllocator,
) -> Result<(), MapError> {
    let phys = vdso_physical_address();
    page_table.map(
        VirtualAddress::new(VDSO_BASE_VA as u64),
        phys,
        PAGE_FLAGS_USER_CODE,
        allocator,
    )
}

/// Map the vDSO data page read-only into `page_table` at `VDSO_DATA_VA`.
///
/// Called from `load_elf()` and `fork()` alongside `vdso_map_into_process`.
/// The physical page is shared (read-only) across all processes.
///
/// # Safety
/// Must be called with IRQs disabled. `allocator` must be the live physical
/// allocator.
pub unsafe fn vdso_map_data_into_process(
    page_table: &mut PageTable,
    allocator: &mut PhysicalAllocator,
) -> Result<(), MapError> {
    let phys = vdso_data_physical_address();
    page_table.map(
        VirtualAddress::new(VDSO_DATA_VA as u64),
        phys,
        PAGE_FLAGS_USER_DATA_READ_ONLY,
        allocator,
    )
}
