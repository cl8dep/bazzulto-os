// platform/qemu_virt/virtio_mmio.rs — VirtIO MMIO bus enumeration.
//
// QEMU virt: 32 virtio-mmio slots at physical 0x0A000000, each 0x200 bytes.
// IRQ for slot N = GIC SPI (16+N) = INTID (48+N).
// Reference: QEMU hw/arm/virt.c

use core::ptr::read_volatile;

// ---------------------------------------------------------------------------
// Platform constants
// ---------------------------------------------------------------------------

pub const VIRTIO_MMIO_BASE: u64 = 0x0A000000;
pub const VIRTIO_MMIO_STRIDE: u64 = 0x200;
pub const VIRTIO_MMIO_COUNT: usize = 32;

// VirtIO MMIO register offsets — virtio spec 1.1 §4.2.2
const REG_MAGIC:     u32 = 0x000;
const REG_VERSION:   u32 = 0x004;
const REG_DEVICE_ID: u32 = 0x008;

// VirtIO magic value — little-endian "virt"
const VIRTIO_MAGIC: u32 = 0x74726976;

// ---------------------------------------------------------------------------
// Scan result state
// ---------------------------------------------------------------------------

struct SlotInfo {
    physical_base: u64,
    slot_index: u32,
    device_id: u32,
}

struct VirtioMmioState {
    slots: [SlotInfo; VIRTIO_MMIO_COUNT],
    count: usize,
    hhdm_offset: u64,
}

impl VirtioMmioState {
    const fn zeroed() -> Self {
        const EMPTY_SLOT: SlotInfo = SlotInfo {
            physical_base: 0,
            slot_index: 0,
            device_id: 0,
        };
        Self {
            slots: [EMPTY_SLOT; VIRTIO_MMIO_COUNT],
            count: 0,
            hhdm_offset: 0,
        }
    }
}

struct SyncCell<T>(core::cell::UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static STATE: SyncCell<VirtioMmioState> = SyncCell(core::cell::UnsafeCell::new(VirtioMmioState::zeroed()));

// ---------------------------------------------------------------------------
// Register read helper
// ---------------------------------------------------------------------------

#[inline]
unsafe fn mmio_read(hhdm_offset: u64, physical_base: u64, offset: u32) -> u32 {
    let virt = (hhdm_offset + physical_base + offset as u64) as *const u32;
    read_volatile(virt)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all 32 virtio-mmio slots and record devices present.
///
/// # Safety
/// Must be called after the HHDM is mapped, from EL1.
pub unsafe fn enumerate(hhdm_offset: u64) {
    let state = &mut *STATE.0.get();
    state.hhdm_offset = hhdm_offset;
    state.count = 0;

    for slot in 0..VIRTIO_MMIO_COUNT {
        let physical_base = VIRTIO_MMIO_BASE + slot as u64 * VIRTIO_MMIO_STRIDE;

        let magic     = mmio_read(hhdm_offset, physical_base, REG_MAGIC);
        let version   = mmio_read(hhdm_offset, physical_base, REG_VERSION);
        let device_id = mmio_read(hhdm_offset, physical_base, REG_DEVICE_ID);

        if magic != VIRTIO_MAGIC {
            continue;
        }
        if version != 1 && version != 2 {
            continue;
        }
        if device_id == 0 {
            continue;
        }

        let index = state.count;
        state.slots[index].physical_base = physical_base;
        state.slots[index].slot_index = slot as u32;
        state.slots[index].device_id = device_id;
        state.count += 1;

        crate::drivers::uart::puts("[virtio] slot found, device_id=");
        crate::drivers::uart::put_hex(device_id as u64);
        crate::drivers::uart::puts("\r\n");
    }
}

/// Find a device by ID. Returns (physical_base, slot_index) or None.
pub fn find_device(device_id: u32) -> Option<(u64, u32)> {
    find_device_by_index(device_id, 0)
}

/// Return the N-th device with the given device_id (0-indexed).
///
/// Used to enumerate multiple devices of the same type (e.g. two virtio-blk
/// instances).  `instance = 0` returns the first match, `instance = 1` the
/// second, and so on.
pub fn find_device_by_index(device_id: u32, instance: usize) -> Option<(u64, u32)> {
    let state = unsafe { &*STATE.0.get() };
    let mut count = 0usize;
    for i in 0..state.count {
        if state.slots[i].device_id == device_id {
            if count == instance {
                return Some((state.slots[i].physical_base, state.slots[i].slot_index));
            }
            count += 1;
        }
    }
    None
}

/// Return the stored HHDM offset (set during enumerate).
pub fn hhdm_offset() -> u64 {
    unsafe { (*STATE.0.get()).hhdm_offset }
}
