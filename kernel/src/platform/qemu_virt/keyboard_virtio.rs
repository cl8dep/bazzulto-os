// platform/qemu_virt/keyboard_virtio.rs — VirtIO input keyboard driver.
//
// Protocol: virtio spec 1.1, §5.8 (Input Device) and §2.7 (Virtqueues).
// The keyboard presents one virtqueue (eventq, index 0). We pre-populate it
// with KEYBOARD_VIRTQUEUE_SIZE write-only descriptors. When the user presses
// a key, the device fills one event buffer and moves the descriptor to the
// used ring. The IRQ handler drains the used ring, translates the event to
// a character, and feeds it to the TTY layer via tty_receive_char().
//
// IRQ routing: QEMU virt wires virtio-mmio slot N to GIC SPI (16+N) = INTID (48+N).

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use super::virtio_mmio;

// ---------------------------------------------------------------------------
// VirtIO MMIO register offsets (legacy v1)
// ---------------------------------------------------------------------------

const REG_STATUS:               u32 = 0x070;
const REG_QUEUE_SEL:            u32 = 0x030;
const REG_QUEUE_NUM_MAX:        u32 = 0x034;
const REG_QUEUE_NUM:            u32 = 0x038;
const REG_QUEUE_NOTIFY:         u32 = 0x050;
const REG_INTERRUPT_STATUS:     u32 = 0x060;
const REG_INTERRUPT_ACK:        u32 = 0x064;
const REG_LEGACY_HOST_FEATURES:   u32 = 0x010;
const REG_LEGACY_GUEST_FEATURES:  u32 = 0x020;
const REG_LEGACY_GUEST_PAGE_SIZE: u32 = 0x028;
const REG_LEGACY_QUEUE_ALIGN:   u32 = 0x03C;
const REG_LEGACY_QUEUE_PFN:     u32 = 0x040;

const STATUS_ACKNOWLEDGE: u32 = 1 << 0;
const STATUS_DRIVER:      u32 = 1 << 1;
const STATUS_DRIVER_OK:   u32 = 1 << 2;
const STATUS_FAILED:      u32 = 1 << 7;

const VIRTQ_DESC_FLAG_WRITE: u16 = 1 << 1;

const VIRTIO_MMIO_INT_VRING: u32 = 1 << 0;

const KEYBOARD_VIRTQUEUE_SIZE: usize = 16;
const KEYBOARD_VIRTQUEUE_ALIGN: u32 = 256;
const PAGE_SIZE: u64 = 4096;

const IRQ_VIRTIO_MMIO_BASE: u32 = 48;

// virtio-input EV_KEY event type
const EV_KEY: u16 = 1;

// Linux evdev key event values
const KEY_EVENT_UP:     u32 = 0;
const KEY_EVENT_DOWN:   u32 = 1;
const KEY_EVENT_REPEAT: u32 = 2;

// Linux evdev keycodes for modifiers
const KEYCODE_LEFT_SHIFT:  u16 = 42;
const KEYCODE_RIGHT_SHIFT: u16 = 54;
const KEYCODE_CAPS_LOCK:   u16 = 58;
const KEYCODE_LEFT_CTRL:   u16 = 29;
const KEYCODE_RIGHT_CTRL:  u16 = 97;
const KEYCODE_LEFT_ALT:    u16 = 56;
const KEYCODE_RIGHT_ALT:   u16 = 100; // AltGr

// ---------------------------------------------------------------------------
// Virtqueue structures
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
    ring:       [u16; KEYBOARD_VIRTQUEUE_SIZE],
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
    ring:        [VirtqUsedElement; KEYBOARD_VIRTQUEUE_SIZE],
    avail_event: u16,
}

#[repr(C)]
struct VirtioInputEvent {
    event_type: u16,
    code:       u16,
    value:      u32,
}

// Layout: descriptors(256) + avail(38) + pad(218) + used(134) + events(128) = 774 bytes
const KEYBOARD_PADDING_SIZE: usize = 218;

#[repr(C)]
struct KeyboardVirtqueue {
    descriptor_table: [VirtqDescriptor; KEYBOARD_VIRTQUEUE_SIZE],   // offset 0
    available_ring:   VirtqAvailableRing,                             // offset 256
    alignment_pad:    [u8; KEYBOARD_PADDING_SIZE],                   // offset 294
    used_ring:        VirtqUsedRing,                                  // offset 512
    event_buffers:    [VirtioInputEvent; KEYBOARD_VIRTQUEUE_SIZE],    // offset 646
}

// ---------------------------------------------------------------------------
// US QWERTY keymap (minimal; 128 entries indexed by Linux evdev keycode)
// Each entry: (normal, shifted)
// ---------------------------------------------------------------------------

// Maps evdev keycode → (normal_char, shifted_char, ctrl_char_or_0)
// 0 = no mapping.
const KEYMAP_US: [(u8, u8); 128] = {
    let mut map = [(0u8, 0u8); 128];
    // Row 1: number row
    // keycode 1 = ESC
    // 2..=11 = 1..=0
    // 12 = minus, 13 = equals, 14 = backspace, 15 = tab
    // Row 2
    // 16=q 17=w 18=e 19=r 20=t 21=y 22=u 23=i 24=o 25=p
    // 26=[ 27=] 28=enter
    // Row 3
    // 30=a 31=s 32=d 33=f 34=g 35=h 36=j 37=k 38=l
    // 39=; 40=' 41=` 43=\
    // Row 4
    // 44=z 45=x 46=c 47=v 48=b 49=n 50=m
    // 51=, 52=. 53=/
    // Space = 57
    // We build this at const time.
    map[1]  = (0x1b, 0x1b);  // ESC
    map[2]  = (b'1', b'!');
    map[3]  = (b'2', b'@');
    map[4]  = (b'3', b'#');
    map[5]  = (b'4', b'$');
    map[6]  = (b'5', b'%');
    map[7]  = (b'6', b'^');
    map[8]  = (b'7', b'&');
    map[9]  = (b'8', b'*');
    map[10] = (b'9', b'(');
    map[11] = (b'0', b')');
    map[12] = (b'-', b'_');
    map[13] = (b'=', b'+');
    map[14] = (0x08, 0x08); // backspace
    map[15] = (b'\t', b'\t');
    map[16] = (b'q', b'Q');
    map[17] = (b'w', b'W');
    map[18] = (b'e', b'E');
    map[19] = (b'r', b'R');
    map[20] = (b't', b'T');
    map[21] = (b'y', b'Y');
    map[22] = (b'u', b'U');
    map[23] = (b'i', b'I');
    map[24] = (b'o', b'O');
    map[25] = (b'p', b'P');
    map[26] = (b'[', b'{');
    map[27] = (b']', b'}');
    map[28] = (b'\n', b'\n');
    map[30] = (b'a', b'A');
    map[31] = (b's', b'S');
    map[32] = (b'd', b'D');
    map[33] = (b'f', b'F');
    map[34] = (b'g', b'G');
    map[35] = (b'h', b'H');
    map[36] = (b'j', b'J');
    map[37] = (b'k', b'K');
    map[38] = (b'l', b'L');
    map[39] = (b';', b':');
    map[40] = (b'\'', b'"');
    map[41] = (b'`', b'~');
    map[43] = (b'\\', b'|');
    map[44] = (b'z', b'Z');
    map[45] = (b'x', b'X');
    map[46] = (b'c', b'C');
    map[47] = (b'v', b'V');
    map[48] = (b'b', b'B');
    map[49] = (b'n', b'N');
    map[50] = (b'm', b'M');
    map[51] = (b',', b'<');
    map[52] = (b'.', b'>');
    map[53] = (b'/', b'?');
    map[57] = (b' ', b' ');
    map
};

// ---------------------------------------------------------------------------
// Driver state
// ---------------------------------------------------------------------------

struct KeyboardState {
    virtqueue_virt:  u64,
    virtqueue_phys:  u64,
    mmio_virt_base:  u64,
    last_used_index: u16,
    irq_intid:       u32,
    shift_held:      bool,
    ctrl_held:       bool,
    capslock_active: bool,
    initialized:     bool,
}

impl KeyboardState {
    const fn uninit() -> Self {
        Self {
            virtqueue_virt: 0,
            virtqueue_phys: 0,
            mmio_virt_base: 0,
            last_used_index: 0,
            irq_intid: 0,
            shift_held: false,
            ctrl_held: false,
            capslock_active: false,
            initialized: false,
        }
    }
}

struct SyncCell<T>(core::cell::UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static KEYBOARD_STATE: SyncCell<KeyboardState> = SyncCell(core::cell::UnsafeCell::new(KeyboardState::uninit()));

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
// Key event handling
// ---------------------------------------------------------------------------

unsafe fn handle_key_event(state: &mut KeyboardState, keycode: u16, event_value: u32) {
    // Update modifier state on both key-down and key-up.
    match keycode {
        KEYCODE_LEFT_SHIFT | KEYCODE_RIGHT_SHIFT => {
            state.shift_held = event_value != KEY_EVENT_UP;
            return;
        }
        KEYCODE_LEFT_CTRL | KEYCODE_RIGHT_CTRL => {
            state.ctrl_held = event_value != KEY_EVENT_UP;
            return;
        }
        KEYCODE_RIGHT_ALT | KEYCODE_LEFT_ALT => {
            return; // AltGr not handled in this minimal driver
        }
        KEYCODE_CAPS_LOCK => {
            if event_value == KEY_EVENT_DOWN {
                state.capslock_active = !state.capslock_active;
            }
            return;
        }
        _ => {}
    }

    // Only emit on key-down or autorepeat.
    if event_value == KEY_EVENT_UP {
        return;
    }

    let keycode_idx = keycode as usize;
    if keycode_idx >= 128 {
        return;
    }

    let (normal, shifted) = KEYMAP_US[keycode_idx];
    if normal == 0 {
        return;
    }

    let mut character = if state.shift_held { shifted } else { normal };

    // Apply caps lock for ASCII letters.
    if state.capslock_active {
        if character >= b'a' && character <= b'z' {
            character = character - b'a' + b'A';
        } else if character >= b'A' && character <= b'Z' {
            character = character - b'A' + b'a';
        }
    }

    // Ctrl+letter → control code.
    if state.ctrl_held {
        if character >= b'a' && character <= b'z' {
            character = character - b'a' + 1;
        } else if character >= b'A' && character <= b'Z' {
            character = character - b'A' + 1;
        }
    }

    // Deliver to TTY layer.
    crate::drivers::tty::tty_receive_char(character);
}

// ---------------------------------------------------------------------------
// IRQ handler
// ---------------------------------------------------------------------------

pub unsafe fn keyboard_irq_handler() {
    let state = &mut *KEYBOARD_STATE.0.get();
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

    let vq = state.virtqueue_virt as *mut KeyboardVirtqueue;

    loop {
        let used_idx = read_volatile(&(*vq).used_ring.idx);
        if state.last_used_index == used_idx {
            break;
        }

        let position = (state.last_used_index as usize) % KEYBOARD_VIRTQUEUE_SIZE;
        let descriptor_index = read_volatile(&(*vq).used_ring.ring[position].identifier) as usize;

        let event = &(*vq).event_buffers[descriptor_index] as *const VirtioInputEvent;
        let event_type = read_volatile(&(*event).event_type);
        let code       = read_volatile(&(*event).code);
        let value      = read_volatile(&(*event).value);

        if event_type == EV_KEY {
            handle_key_event(state, code, value);
        }

        // Return descriptor to available ring.
        let avail_ring = &mut (*vq).available_ring as *mut VirtqAvailableRing;
        let avail_idx = read_volatile(&(*avail_ring).idx);
        let avail_slot = (avail_idx as usize) % KEYBOARD_VIRTQUEUE_SIZE;
        write_volatile(&mut (*avail_ring).ring[avail_slot], descriptor_index as u16);

        fence(Ordering::Release);
        write_volatile(&mut (*avail_ring).idx, avail_idx.wrapping_add(1));

        state.last_used_index = state.last_used_index.wrapping_add(1);
    }

    // Notify the device that the available ring has new entries.
    fence(Ordering::Release);
    mmio_write(base, REG_QUEUE_NOTIFY, 0);
}

pub fn keyboard_get_irq_id() -> u32 {
    unsafe { (*KEYBOARD_STATE.0.get()).irq_intid }
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

pub unsafe fn keyboard_init(hhdm_offset: u64) {
    let state = &mut *KEYBOARD_STATE.0.get();
    *state = KeyboardState::uninit();

    // Find virtio-input device (DeviceID 18).
    let (physical_base, slot) = match virtio_mmio::find_device(18) {
        Some(pair) => pair,
        None => {
            crate::drivers::uart::puts("[keyboard] no virtio-input device found\r\n");
            return;
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

    // Step 4: Feature negotiation
    let _ = mmio_read(base, REG_LEGACY_HOST_FEATURES);
    mmio_write(base, REG_LEGACY_GUEST_FEATURES, 0);

    // Step 5: Guest page size
    mmio_write(base, REG_LEGACY_GUEST_PAGE_SIZE, PAGE_SIZE as u32);

    // Step 6: Allocate virtqueue page.
    let queue_phys = crate::memory::with_physical_allocator(|phys| {
        phys.alloc().map(|pa| pa.as_u64())
    });

    let queue_phys = match queue_phys {
        Some(p) => p,
        None => {
            crate::drivers::uart::puts("[keyboard] failed to alloc virtqueue page\r\n");
            mmio_write(base, REG_STATUS, STATUS_FAILED);
            return;
        }
    };

    let queue_virt = hhdm_offset + queue_phys;
    core::ptr::write_bytes(queue_virt as *mut u8, 0, PAGE_SIZE as usize);

    state.virtqueue_phys = queue_phys;
    state.virtqueue_virt = queue_virt;

    mmio_write(base, REG_QUEUE_SEL, 0);

    let queue_num_max = mmio_read(base, REG_QUEUE_NUM_MAX);
    if queue_num_max < KEYBOARD_VIRTQUEUE_SIZE as u32 {
        crate::drivers::uart::puts("[keyboard] device queue too small\r\n");
        mmio_write(base, REG_STATUS, STATUS_FAILED);
        return;
    }

    mmio_write(base, REG_QUEUE_NUM, KEYBOARD_VIRTQUEUE_SIZE as u32);
    mmio_write(base, REG_LEGACY_QUEUE_ALIGN, KEYBOARD_VIRTQUEUE_ALIGN);
    mmio_write(base, REG_LEGACY_QUEUE_PFN, (queue_phys / PAGE_SIZE) as u32);

    // Pre-populate all descriptors. Each points to one event buffer, device-writable.
    let vq = queue_virt as *mut KeyboardVirtqueue;
    let events_offset = core::mem::offset_of!(KeyboardVirtqueue, event_buffers) as u64;

    for i in 0..KEYBOARD_VIRTQUEUE_SIZE {
        let event_phys = queue_phys + events_offset + i as u64 * core::mem::size_of::<VirtioInputEvent>() as u64;

        let desc = &mut (*vq).descriptor_table[i] as *mut VirtqDescriptor;
        write_volatile(&mut (*desc).address, event_phys);
        write_volatile(&mut (*desc).length, core::mem::size_of::<VirtioInputEvent>() as u32);
        write_volatile(&mut (*desc).flags, VIRTQ_DESC_FLAG_WRITE);
        write_volatile(&mut (*desc).next, 0);

        let avail_ring = &mut (*vq).available_ring as *mut VirtqAvailableRing;
        write_volatile(&mut (*avail_ring).ring[i], i as u16);
    }

    fence(Ordering::Release);
    let avail_ring = &mut (*vq).available_ring as *mut VirtqAvailableRing;
    write_volatile(&mut (*avail_ring).idx, KEYBOARD_VIRTQUEUE_SIZE as u16);

    // Step 7: Enable GIC interrupt.
    state.irq_intid = IRQ_VIRTIO_MMIO_BASE + slot;
    crate::platform::qemu_virt::gic_enable_interrupt(state.irq_intid);

    // Step 8: DRIVER_OK
    mmio_write(base, REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK);

    state.initialized = true;

    crate::drivers::uart::puts("[keyboard] virtio-input initialized, INTID=");
    crate::drivers::uart::put_hex(state.irq_intid as u64);
    crate::drivers::uart::puts("\r\n");
}
