// Limine bootloader protocol bindings — revision 0
// Source: limine/limine.h (BSD Zero Clause License)
//
// All structs are #[repr(C)] to match the layout that Limine writes into memory.
// Requests are placed in .limine_requests via #[link_section] so Limine can find them.

// ---------------------------------------------------------------------------
// Magic numbers
// ---------------------------------------------------------------------------

const COMMON_MAGIC_0: u64 = 0xc7b1dd30df4c8b88;
const COMMON_MAGIC_1: u64 = 0x0a82e883a194f07b;

// ---------------------------------------------------------------------------
// Framebuffer request
// ---------------------------------------------------------------------------

pub const FRAMEBUFFER_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0x9d5827dcd881dd75,
    0xa3148604f6fab11b,
];

#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u32,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub _unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *mut u8,
    pub mode_count: u64,
    pub modes: *mut *mut u8,
}

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *mut *mut Framebuffer,
}

#[repr(C)]
pub struct FramebufferRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut FramebufferResponse,
}

unsafe impl Sync for FramebufferRequest {}
unsafe impl Sync for FramebufferResponse {}
unsafe impl Sync for Framebuffer {}

// ---------------------------------------------------------------------------
// HHDM request
// ---------------------------------------------------------------------------

pub const HHDM_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0x48dcf1cb8ad2b852,
    0x63984e959a98244b,
];

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct HhdmRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut HhdmResponse,
}

unsafe impl Sync for HhdmRequest {}

// ---------------------------------------------------------------------------
// Memory map request
// ---------------------------------------------------------------------------

pub const MEMMAP_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0x67cf3d9d378a806f,
    0xe304acdfc50c3c62,
];

pub const MEMMAP_USABLE: u64 = 0;
pub const MEMMAP_RESERVED: u64 = 1;
pub const MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
pub const MEMMAP_ACPI_NVS: u64 = 3;
pub const MEMMAP_BAD_MEMORY: u64 = 4;
pub const MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
pub const MEMMAP_KERNEL_AND_MODULES: u64 = 6;
pub const MEMMAP_FRAMEBUFFER: u64 = 7;

#[repr(C)]
pub struct MemmapEntry {
    pub base: u64,
    pub length: u64,
    pub entry_type: u64,
}

#[repr(C)]
pub struct MemmapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *mut *mut MemmapEntry,
}

#[repr(C)]
pub struct MemmapRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut MemmapResponse,
}

unsafe impl Sync for MemmapRequest {}
unsafe impl Sync for MemmapResponse {}

// ---------------------------------------------------------------------------
// Kernel address request
// ---------------------------------------------------------------------------

pub const KERNEL_ADDRESS_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0x71ba76863cc55f63,
    0xb2644a48c516a487,
];

#[repr(C)]
pub struct KernelAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

#[repr(C)]
pub struct KernelAddressRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut KernelAddressResponse,
}

unsafe impl Sync for KernelAddressRequest {}

// ---------------------------------------------------------------------------
// Bootloader info request
// ---------------------------------------------------------------------------

pub const BOOTLOADER_INFO_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0xf55038d8e2a1202f,
    0x279426fcf5f59740,
];

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    pub name: *const u8,
    pub version: *const u8,
}

#[repr(C)]
pub struct BootloaderInfoRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut BootloaderInfoResponse,
}

unsafe impl Sync for BootloaderInfoRequest {}

// ---------------------------------------------------------------------------
// Device Tree Blob request
// ---------------------------------------------------------------------------

// Device Tree Blob request (Limine protocol)
// Magic from: https://github.com/limine-bootloader/limine/blob/stable-v8.x/limine.h
pub const DEVICE_TREE_BLOB_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0xb40ddb48fb54bac7,
    0x545081493f81ffb7,
];

#[repr(C)]
pub struct DeviceTreeBlobResponse {
    pub revision: u64,
    pub address: *mut u8,
}

#[repr(C)]
pub struct DeviceTreeBlobRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut DeviceTreeBlobResponse,
}

unsafe impl Sync for DeviceTreeBlobRequest {}
unsafe impl Sync for DeviceTreeBlobResponse {}

// ---------------------------------------------------------------------------
// SMP (Symmetric Multi-Processing) request
// ---------------------------------------------------------------------------

/// Limine SMP request magic numbers (AArch64 variant).
///
/// Source: limine/limine.h, limine_smp_request (AArch64).
pub const SMP_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0x95a67b819a1b857e,
    0xa0b61b723b6a73e0,
];

/// Per-CPU information structure filled in by Limine during SMP bringup.
///
/// Limine allocates one of these for each logical CPU and places them in an
/// array pointed to by `SmpResponse::cpus`.
///
/// To wake an Application Processor (AP), the kernel writes the entry-point
/// function pointer into `goto_address` using a volatile store.  Limine polls
/// this field with a load-acquire and jumps to it once it becomes non-null.
///
/// Source: limine/limine.h `struct limine_smp_info`.
#[repr(C)]
pub struct SmpCpuInfo {
    /// ACPI processor UID (not used on AArch64; always 0 on QEMU virt).
    pub processor_id: u32,
    /// GIC CPU interface number for this core.
    pub gic_iface_no: u32,
    /// MPIDR_EL1 value for this CPU (affinity fields).
    pub mpidr: u64,
    /// Reserved; must not be written.
    pub reserved: u64,
    /// Written by the kernel to wake this AP.
    ///
    /// Limine polls this field atomically and jumps to the written address
    /// once it becomes non-null.  Write with `write_volatile`.
    pub goto_address: *const (),
    /// Arbitrary value passed to the AP entry function.
    ///
    /// We store the `cpu_id` (0-based index) here so `ap_entry` can
    /// retrieve it without additional global state.
    pub extra_argument: u64,
}

// SAFETY: SmpCpuInfo is only written once (goto_address) by the BSP during
// SMP bringup, and each AP reads its own entry exactly once.
unsafe impl Sync for SmpCpuInfo {}
unsafe impl Send for SmpCpuInfo {}

/// Limine SMP response, written by the bootloader.
#[repr(C)]
pub struct SmpResponse {
    pub revision: u64,
    /// Bit 0: X2APIC enabled (x86 only; always 0 on AArch64).
    pub flags: u32,
    /// MPIDR_EL1 of the bootstrap processor (BSP).
    pub bsp_mpidr: u64,
    /// Number of entries in `cpus`.
    pub cpu_count: u64,
    /// Pointer to an array of `cpu_count` pointers, each pointing to an
    /// `SmpCpuInfo` for one logical CPU.
    pub cpus: *mut *mut SmpCpuInfo,
}

// SAFETY: written by Limine before kernel entry; read-only after that
// (except for goto_address which is write_volatile from BSP only).
unsafe impl Sync for SmpResponse {}

/// Limine SMP request placed in `.limine_requests`.
#[repr(C)]
pub struct SmpRequest {
    pub id: [u64; 4],
    pub revision: u64,
    /// Flags: bit 0 = enable X2APIC (x86 only; set to 0 on AArch64).
    pub flags: u64,
    /// Written by Limine with a pointer to the SMP response.
    pub response: *mut SmpResponse,
}

// SAFETY: the response field is written once by Limine before kernel_main runs.
unsafe impl Sync for SmpRequest {}

// ---------------------------------------------------------------------------
// Kernel file request — gives access to the kernel's own limine_file entry,
// including the `cmdline` field which carries boot parameters like `root=`.
//
// Magic: LIMINE_KERNEL_FILE_REQUEST / LIMINE_EXECUTABLE_FILE_REQUEST
// Source: limine/limine.h, LIMINE_API_REVISION < 2 branch.
// ---------------------------------------------------------------------------

pub const KERNEL_FILE_REQUEST_ID: [u64; 4] = [
    COMMON_MAGIC_0,
    COMMON_MAGIC_1,
    0xad97e90e83f1ed67,
    0x31eb5d1c5ff23b69,
];

/// UUID from limine_file (used for partition identification).
#[repr(C)]
pub struct LimineUuid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

/// A file loaded by Limine (module or kernel image).
///
/// Source: limine/limine.h `struct limine_file`.
#[repr(C)]
pub struct LimineFile {
    pub revision: u64,
    pub address: *mut u8,
    pub size: u64,
    pub path: *const u8,
    /// Kernel command-line string, null-terminated.
    /// Set via `cmdline:` in limine.conf for the boot entry.
    pub cmdline: *const u8,
    pub media_type: u32,
    pub unused: u32,
    pub tftp_ip: u32,
    pub tftp_port: u32,
    pub partition_index: u32,
    pub mbr_disk_id: u32,
    pub gpt_disk_uuid: LimineUuid,
    pub gpt_part_uuid: LimineUuid,
    pub part_uuid: LimineUuid,
}

#[repr(C)]
pub struct KernelFileResponse {
    pub revision: u64,
    pub kernel_file: *mut LimineFile,
}

#[repr(C)]
pub struct KernelFileRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut KernelFileResponse,
}

unsafe impl Sync for LimineFile {}
unsafe impl Sync for KernelFileResponse {}
unsafe impl Sync for KernelFileRequest {}
