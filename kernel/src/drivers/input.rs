// drivers/input.rs — Input ring buffer abstraction.
//
// Single-producer (IRQ handlers) / single-consumer (sys_read on TTY) ring
// buffer. All character sources (keyboard, UART) write here via emit_char().
//
// Port of kernel/drivers/input/input.c.

use core::sync::atomic::{AtomicU32, Ordering};

const INPUT_RING_SIZE: usize = 64;

/// SIGINT signal number.
const SIGINT: u8 = 2;

// ---------------------------------------------------------------------------
// Ring buffer state
// ---------------------------------------------------------------------------

struct InputRing {
    buffer: [u8; INPUT_RING_SIZE],
    head: AtomicU32, // ISR writes here
    tail: AtomicU32, // reader reads here
}

impl InputRing {
    const fn new() -> Self {
        Self {
            buffer: [0u8; INPUT_RING_SIZE],
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }
}

struct SyncInputRing(core::cell::UnsafeCell<InputRing>);
unsafe impl Sync for SyncInputRing {}

static RING: SyncInputRing = SyncInputRing(core::cell::UnsafeCell::new(InputRing::new()));

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn init() {
    let ring = unsafe { &mut *RING.0.get() };
    ring.head.store(0, Ordering::Relaxed);
    ring.tail.store(0, Ordering::Relaxed);
}

/// Called from IRQ handlers. Enqueues a char; Ctrl+C → SIGINT to foreground.
pub fn emit_char(character: u8) {
    if character == 0x03 {
        // Ctrl+C: send SIGINT to foreground process, do not enqueue.
        unsafe {
            crate::drivers::uart::puts("^C\r\n");
            send_sigint_to_foreground();
        }
        return;
    }

    let ring = unsafe { &mut *RING.0.get() };
    let head = ring.head.load(Ordering::Relaxed);
    let next_head = (head + 1) % INPUT_RING_SIZE as u32;

    if next_head == ring.tail.load(Ordering::Acquire) {
        return; // ring full — drop character
    }

    ring.buffer[head as usize] = character;
    ring.head.store(next_head, Ordering::Release);
}

/// Called from sys_read on TTY. Blocks until a character is available.
/// Returns the character on success.
pub fn getchar() -> u8 {
    let ring = unsafe { &*RING.0.get() };

    // Disable IRQs to prevent race between checking ring and sleeping.
    unsafe { core::arch::asm!("msr daifset, #2", options(nostack, nomem)) };

    loop {
        let head = ring.head.load(Ordering::Acquire);
        let tail = ring.tail.load(Ordering::Relaxed);

        if head != tail {
            break;
        }

        // Yield to let other processes run while waiting.
        unsafe {
            core::arch::asm!("msr daifclr, #2", options(nostack, nomem));
            crate::scheduler::with_scheduler(|scheduler| {
                scheduler.schedule();
            });
            core::arch::asm!("msr daifset, #2", options(nostack, nomem));
        }
    }

    let tail = ring.tail.load(Ordering::Relaxed);
    let character = ring.buffer[tail as usize];
    ring.tail.store((tail + 1) % INPUT_RING_SIZE as u32, Ordering::Release);

    unsafe { core::arch::asm!("msr daifclr, #2", options(nostack, nomem)) };

    character
}

unsafe fn send_sigint_to_foreground() {
    unsafe { crate::scheduler::with_scheduler(|scheduler| {
        for slot_index in 0..crate::scheduler::PID_MAX {
            let pid = crate::process::Pid::new(slot_index as u16, 1);
            if let Some(process) = scheduler.process(pid) {
                if process.is_foreground {
                    let foreground_pid = process.pid;
                    scheduler.send_signal_to(foreground_pid, SIGINT);
                    return;
                }
            }
        }
    }); }
}
