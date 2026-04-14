// platform/qemu_virt/virtio_blk.rs — VirtIO block device driver.
//
// Uses virtio-mmio legacy (v1) transport with a single requestq (queue 0).
// Each I/O operation is a 3-descriptor chain: header → data → status.
// I/O is synchronous: busy-waits with WFI until the IRQ fires.
//
// Reference: virtio spec 1.1 §5.2, §4.2.3, §2.7.
// IRQ routing: QEMU virt wires virtio-mmio slot N to GIC SPI (16+N) = INTID (48+N).

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use super::virtio_mmio;

// ---------------------------------------------------------------------------
// VirtIO MMIO register offsets (legacy v1)
// ---------------------------------------------------------------------------

const REG_STATUS:              u32 = 0x070;
const REG_QUEUE_SEL:           u32 = 0x030;
const REG_QUEUE_NUM_MAX:       u32 = 0x034;
const REG_QUEUE_NUM:           u32 = 0x038;
const REG_QUEUE_NOTIFY:        u32 = 0x050;
const REG_INTERRUPT_STATUS:    u32 = 0x060;
const REG_INTERRUPT_ACK:       u32 = 0x064;
const REG_LEGACY_HOST_FEATURES:  u32 = 0x010;
const REG_LEGACY_GUEST_FEATURES: u32 = 0x020;
const REG_LEGACY_GUEST_PAGE_SIZE: u32 = 0x028;
const REG_LEGACY_QUEUE_ALIGN:  u32 = 0x03C;
const REG_LEGACY_QUEUE_PFN:    u32 = 0x040;
const REG_CONFIG_CAPACITY_LO:  u32 = 0x100;
const REG_CONFIG_CAPACITY_HI:  u32 = 0x104;

// Device status bits
const STATUS_ACKNOWLEDGE: u32 = 1 << 0;
const STATUS_DRIVER:      u32 = 1 << 1;
const STATUS_DRIVER_OK:   u32 = 1 << 2;
const STATUS_FAILED:      u32 = 1 << 7;

// Descriptor flags
const VIRTQ_DESC_FLAG_NEXT:  u16 = 1 << 0;
const VIRTQ_DESC_FLAG_WRITE: u16 = 1 << 1;

// Block request types
const VIRTIO_BLK_TYPE_IN:  u32 = 0; // device → driver (read)
const VIRTIO_BLK_TYPE_OUT: u32 = 1; // driver → device (write)

// Interrupt status flags
const VIRTIO_MMIO_INT_VRING: u32 = 1 << 0;

const BLK_VIRTQUEUE_SIZE: usize = 16;
const BLK_VIRTQUEUE_ALIGN: u32 = 256;
const PAGE_SIZE: u64 = 4096;

const IRQ_VIRTIO_MMIO_BASE: u32 = 48;

// ---------------------------------------------------------------------------
// Virtqueue structures (virtio spec 1.1 §2.7)
// ---------------------------------------------------------------------------

#[repr(C)]
struct VirtqDescriptor {
    address: u64,
    length:  u32,
    flags:   u16,
    next:    u16,
}

#[repr(C)]
struct VirtqAvailableRing {
    flags:      u16,
    idx:        u16,
    ring:       [u16; BLK_VIRTQUEUE_SIZE],
    used_event: u16,
}

#[repr(C)]
struct VirtqUsedElement {
    identifier: u32,
    length:     u32,
}

#[repr(C)]
struct VirtqUsedRing {
    flags:       u16,
    idx:         u16,
    ring:        [VirtqUsedElement; BLK_VIRTQUEUE_SIZE],
    avail_event: u16,
}

// BLK_PADDING_SIZE = 512 - (sizeof(VirtqAvailableRing)) = 512 - (2+2+16*2+2) = 512 - 38 = 474?
// Wait: with BLK_VIRTQUEUE_SIZE=16:
//   descriptor_table: 16*16 = 256 bytes
//   available_ring: 2+2+(2*16)+2 = 38 bytes → ends at 294
//   alignment_pad: need to reach 512 → 512-294 = 218 bytes
//   used_ring: 2+2+(8*16)+2 = 134 bytes
//   request_header: 16 bytes
//   status_byte: 1 byte
const BLK_PADDING_SIZE: usize = 218;

#[repr(C)]
struct VirtioBlkRequestHeader {
    request_type: u32,
    reserved:     u32,
    sector:       u64,
}

#[repr(C)]
struct BlkVirtqueue {
    descriptor_table: [VirtqDescriptor; BLK_VIRTQUEUE_SIZE],   // offset 0
    available_ring:   VirtqAvailableRing,                        // offset 256
    alignment_pad:    [u8; BLK_PADDING_SIZE],                   // offset 294
    used_ring:        VirtqUsedRing,                             // offset 512
    request_header:   VirtioBlkRequestHeader,                    // after used ring
    status_byte:      u8,
}

// ---------------------------------------------------------------------------
// Driver state
// ---------------------------------------------------------------------------

struct DiskState {
    virtqueue_virt:   u64,   // virtual address of BlkVirtqueue
    virtqueue_phys:   u64,   // physical address of the queue page
    mmio_virt_base:   u64,   // virtual MMIO base
    /// DMA bounce buffer — one page allocated directly from the physical
    /// allocator and accessed via HHDM.  Because it is a raw physical page,
    /// `dma_buf_phys = dma_buf_virt - hhdm_offset` is always correct.
    /// Heap allocations (Vec, Box) are mapped at kernel image VAs, NOT at
    /// HHDM VAs, so the same subtraction would yield garbage physical addresses.
    dma_buf_virt:     u64,
    dma_buf_phys:     u64,
    last_used_index:  u16,
    irq_intid:        u32,
    capacity_sectors: u64,
    initialized:      bool,
}

impl DiskState {
    const fn uninit() -> Self {
        Self {
            virtqueue_virt: 0,
            virtqueue_phys: 0,
            mmio_virt_base: 0,
            dma_buf_virt: 0,
            dma_buf_phys: 0,
            last_used_index: 0,
            irq_intid: 0,
            capacity_sectors: 0,
            initialized: false,
        }
    }
}

/// Maximum number of simultaneously supported virtio-blk instances.
const MAX_BLKS: usize = 4;

struct SyncCell<T>(core::cell::UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static DISK_STATES: SyncCell<[DiskState; MAX_BLKS]> = SyncCell(core::cell::UnsafeCell::new([
    DiskState::uninit(), DiskState::uninit(), DiskState::uninit(), DiskState::uninit(),
]));

// ---------------------------------------------------------------------------
// MMIO helpers
// ---------------------------------------------------------------------------

#[inline]
unsafe fn mmio_read(base: u64, offset: u32) -> u32 {
    read_volatile((base + offset as u64) as *const u32)
}

#[inline]
unsafe fn mmio_write(base: u64, offset: u32, value: u32) {
    write_volatile((base + offset as u64) as *mut u32, value);
}

// ---------------------------------------------------------------------------
// Synchronous I/O
// ---------------------------------------------------------------------------

unsafe fn blk_do_request(
    state: &mut DiskState,
    request_type: u32,
    sector: u64,
    data_buf_virt: u64,
    data_buf_phys: u64,
    data_len: u32,
) -> bool {
    let vq = state.virtqueue_virt as *mut BlkVirtqueue;
    let hhdm = virtio_mmio::hhdm_offset();

    // Physical addresses of header and status within the queue page.
    let header_offset = core::mem::offset_of!(BlkVirtqueue, request_header) as u64;
    let status_offset = core::mem::offset_of!(BlkVirtqueue, status_byte) as u64;
    let header_phys = state.virtqueue_phys + header_offset;
    let status_phys = state.virtqueue_phys + status_offset;

    // Fill request header.
    let _ = data_buf_virt; // used only to pass physical address
    let header_ptr = (state.virtqueue_virt + header_offset) as *mut VirtioBlkRequestHeader;
    write_volatile(&mut (*header_ptr).request_type, request_type);
    write_volatile(&mut (*header_ptr).reserved, 0);
    write_volatile(&mut (*header_ptr).sector, sector);

    // Status sentinel.
    let status_ptr = (state.virtqueue_virt + status_offset) as *mut u8;
    write_volatile(status_ptr, 0xFF);

    // Descriptor 0: request header (device-readable)
    let desc0 = &mut (*vq).descriptor_table[0] as *mut VirtqDescriptor;
    write_volatile(&mut (*desc0).address, header_phys);
    write_volatile(&mut (*desc0).length, core::mem::size_of::<VirtioBlkRequestHeader>() as u32);
    write_volatile(&mut (*desc0).flags, VIRTQ_DESC_FLAG_NEXT);
    write_volatile(&mut (*desc0).next, 1);

    // Descriptor 1: data buffer
    let data_write_flag = if request_type == VIRTIO_BLK_TYPE_IN { VIRTQ_DESC_FLAG_WRITE } else { 0 };
    let desc1 = &mut (*vq).descriptor_table[1] as *mut VirtqDescriptor;
    write_volatile(&mut (*desc1).address, data_buf_phys);
    write_volatile(&mut (*desc1).length, data_len);
    write_volatile(&mut (*desc1).flags, VIRTQ_DESC_FLAG_NEXT | data_write_flag);
    write_volatile(&mut (*desc1).next, 2);

    // Descriptor 2: status byte (device-writable)
    let desc2 = &mut (*vq).descriptor_table[2] as *mut VirtqDescriptor;
    write_volatile(&mut (*desc2).address, status_phys);
    write_volatile(&mut (*desc2).length, 1);
    write_volatile(&mut (*desc2).flags, VIRTQ_DESC_FLAG_WRITE);
    write_volatile(&mut (*desc2).next, 0);

    // Place descriptor chain head (0) in the available ring.
    let avail_ring = &mut (*vq).available_ring as *mut VirtqAvailableRing;
    let avail_idx = read_volatile(&(*avail_ring).idx);
    let avail_slot = (avail_idx as usize) % BLK_VIRTQUEUE_SIZE;
    write_volatile(&mut (*avail_ring).ring[avail_slot], 0u16);

    fence(Ordering::Release);
    write_volatile(&mut (*avail_ring).idx, avail_idx.wrapping_add(1));

    // Notify the device.
    fence(Ordering::Release);
    mmio_write(state.mmio_virt_base, REG_QUEUE_NOTIFY, 0);

    // Wait for device completion using WFI.
    let used_ring = &(*vq).used_ring as *const VirtqUsedRing;
    loop {
        let used_idx = read_volatile(&(*used_ring).idx);
        if state.last_used_index != used_idx {
            state.last_used_index = used_idx;
            break;
        }
        // Enable IRQs, yield CPU to QEMU's host event loop, re-disable.
        core::arch::asm!("msr daifclr, #2", options(nostack, nomem));
        core::arch::asm!("wfi", options(nostack, nomem));
        core::arch::asm!("msr daifset, #2", options(nostack, nomem));
    }

    fence(Ordering::Acquire);

    // Check status byte.
    let status = read_volatile(status_ptr);
    status == 0 // VIRTIO_BLK_STATUS_OK
}

// ---------------------------------------------------------------------------
// Public API (implements hal::disk)
// ---------------------------------------------------------------------------

/// Initialise the N-th virtio-blk instance (0-indexed).
///
/// Returns `true` if a device was found and initialised, `false` if no
/// device at that index exists.  Callers should loop from index 0 upward
/// until this function returns `false`.
///
/// # Safety
/// Must be called after `virtio_mmio::enumerate()` and HHDM mapping.
pub unsafe fn disk_init_instance(hhdm_offset: u64, instance: usize) -> bool {
    if instance >= MAX_BLKS {
        return false;
    }
    let state = &mut (*DISK_STATES.0.get())[instance];
    *state = DiskState::uninit();

    // Find the N-th virtio-blk device (DeviceID 2).
    let (physical_base, slot) = match virtio_mmio::find_device_by_index(2, instance) {
        Some(pair) => pair,
        None => {
            return false;
        }
    };

    state.mmio_virt_base = hhdm_offset + physical_base;
    let base = state.mmio_virt_base;

    // Step 1: Reset
    mmio_write(base, REG_STATUS, 0);

    // Step 2: ACKNOWLEDGE
    mmio_write(base, REG_STATUS, STATUS_ACKNOWLEDGE);

    // Step 3: DRIVER
    mmio_write(base, REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

    // Step 4: Feature negotiation — accept no features.
    let _ = mmio_read(base, REG_LEGACY_HOST_FEATURES);
    mmio_write(base, REG_LEGACY_GUEST_FEATURES, 0);

    // Step 5: Guest page size
    mmio_write(base, REG_LEGACY_GUEST_PAGE_SIZE, PAGE_SIZE as u32);

    // Read device capacity from config space (offset 0x100).
    let cap_lo = mmio_read(base, REG_CONFIG_CAPACITY_LO);
    let cap_hi = mmio_read(base, REG_CONFIG_CAPACITY_HI);
    state.capacity_sectors = ((cap_hi as u64) << 32) | cap_lo as u64;

    // Step 6: Allocate virtqueue page.
    let queue_phys = crate::memory::with_physical_allocator(|phys| {
        phys.alloc().map(|pa| pa.as_u64())
    });

    let queue_phys = match queue_phys {
        Some(p) => p,
        None => {
            crate::drivers::uart::puts("[blk] failed to alloc virtqueue page\r\n");
            mmio_write(base, REG_STATUS, STATUS_FAILED);
            return false;
        }
    };

    let queue_virt = hhdm_offset + queue_phys;

    // Zero the page.
    core::ptr::write_bytes(queue_virt as *mut u8, 0, PAGE_SIZE as usize);

    state.virtqueue_phys = queue_phys;
    state.virtqueue_virt = queue_virt;

    // Allocate a dedicated DMA bounce buffer page.
    // Heap allocations (Vec/Box) live at kernel-image VAs, not HHDM VAs, so
    // subtracting hhdm_offset from a heap pointer yields a wrong physical
    // address.  A raw physical page accessed through HHDM is always correct:
    //   dma_buf_phys = dma_buf_virt - hhdm_offset  ✓
    let dma_phys = crate::memory::with_physical_allocator(|phys| {
        phys.alloc().map(|pa| pa.as_u64())
    });
    let dma_phys = match dma_phys {
        Some(p) => p,
        None => {
            crate::drivers::uart::puts("[blk] failed to alloc DMA bounce buffer\r\n");
            mmio_write(base, REG_STATUS, STATUS_FAILED);
            return false;
        }
    };
    let dma_virt = hhdm_offset + dma_phys;
    state.dma_buf_phys = dma_phys;
    state.dma_buf_virt = dma_virt;

    // Configure queue 0.
    mmio_write(base, REG_QUEUE_SEL, 0);

    let queue_num_max = mmio_read(base, REG_QUEUE_NUM_MAX);
    if queue_num_max < BLK_VIRTQUEUE_SIZE as u32 {
        crate::drivers::uart::puts("[blk] device queue too small\r\n");
        mmio_write(base, REG_STATUS, STATUS_FAILED);
        return false;
    }

    mmio_write(base, REG_QUEUE_NUM, BLK_VIRTQUEUE_SIZE as u32);
    mmio_write(base, REG_LEGACY_QUEUE_ALIGN, BLK_VIRTQUEUE_ALIGN);
    mmio_write(base, REG_LEGACY_QUEUE_PFN, (queue_phys / PAGE_SIZE) as u32);

    // Step 7: Enable GIC interrupt for this slot.
    state.irq_intid = IRQ_VIRTIO_MMIO_BASE + slot;
    crate::platform::qemu_virt::gic_enable_interrupt(state.irq_intid);

    // Step 8: DRIVER_OK
    mmio_write(base, REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK);

    state.initialized = true;

    crate::drivers::uart::puts("[blk] disk");
    crate::drivers::uart::putc(b'a' + instance as u8);
    crate::drivers::uart::puts(" initialized, capacity=");
    crate::drivers::uart::put_hex(state.capacity_sectors);
    crate::drivers::uart::puts(" sectors\r\n");

    true
}

/// Read `count` 512-byte sectors starting at `lba` into `buf`, using disk `disk_index`.
///
/// Uses the pre-allocated DMA bounce buffer (HHDM VA, known physical address)
/// for each sector, then copies the result into the caller's buffer.  This
/// avoids the assumption that the caller's buffer resides in HHDM virtual
/// space — heap allocations (Vec, Box) live at kernel-image VAs and their
/// physical addresses cannot be derived by subtracting hhdm_offset.
pub unsafe fn disk_read_sectors(disk_index: usize, lba: u64, count: u32, buf: &mut [u8]) -> bool {
    if disk_index >= MAX_BLKS { return false; }
    let state = &mut (*DISK_STATES.0.get())[disk_index];
    if !state.initialized || count == 0 {
        return false;
    }
    if buf.len() < count as usize * 512 {
        return false;
    }
    if state.dma_buf_virt == 0 {
        return false;
    }

    let dma_virt = state.dma_buf_virt;
    let dma_phys = state.dma_buf_phys;

    for i in 0..count as usize {
        // Issue the DMA read into the bounce buffer (physical address known).
        if !blk_do_request(state, VIRTIO_BLK_TYPE_IN, lba + i as u64, dma_virt, dma_phys, 512) {
            return false;
        }
        // Copy from bounce buffer into the caller's buffer.
        let dst = buf.as_mut_ptr().add(i * 512);
        let src = dma_virt as *const u8;
        core::ptr::copy_nonoverlapping(src, dst, 512);
    }
    true
}

/// Write `count` 512-byte sectors starting at `lba` from `buf`, using disk `disk_index`.
///
/// Copies each sector into the DMA bounce buffer before issuing the request.
pub unsafe fn disk_write_sectors(disk_index: usize, lba: u64, count: u32, buf: &[u8]) -> bool {
    if disk_index >= MAX_BLKS { return false; }
    let state = &mut (*DISK_STATES.0.get())[disk_index];
    if !state.initialized || count == 0 {
        return false;
    }
    if buf.len() < count as usize * 512 {
        return false;
    }
    if state.dma_buf_virt == 0 {
        return false;
    }

    let dma_virt = state.dma_buf_virt;
    let dma_phys = state.dma_buf_phys;

    for i in 0..count as usize {
        // Copy caller's data into the bounce buffer.
        let src = buf.as_ptr().add(i * 512);
        let dst = dma_virt as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, 512);

        if !blk_do_request(state, VIRTIO_BLK_TYPE_OUT, lba + i as u64, dma_virt, dma_phys, 512) {
            return false;
        }
    }
    true
}

pub fn disk_capacity(disk_index: usize) -> u64 {
    if disk_index >= MAX_BLKS { return 0; }
    unsafe { (*DISK_STATES.0.get())[disk_index].capacity_sectors }
}

pub fn disk_get_irq_id(disk_index: usize) -> u32 {
    if disk_index >= MAX_BLKS { return 0; }
    unsafe { (*DISK_STATES.0.get())[disk_index].irq_intid }
}

pub unsafe fn disk_irq_handler(disk_index: usize) {
    if disk_index >= MAX_BLKS { return; }
    let state = &mut (*DISK_STATES.0.get())[disk_index];
    if !state.initialized {
        return;
    }

    // Acknowledge the interrupt.
    let base = state.mmio_virt_base;
    let interrupt_status = mmio_read(base, REG_INTERRUPT_STATUS);
    mmio_write(base, REG_INTERRUPT_ACK, interrupt_status);

    if interrupt_status & VIRTIO_MMIO_INT_VRING == 0 {
        return;
    }

    fence(Ordering::Acquire);

    // The blk_do_request loop checks used_ring.idx directly; the IRQ merely
    // wakes the WFI. Nothing else to do here.
}
