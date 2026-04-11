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
