//! Bazzulto OS — Rust kernel entry point
//!
//! This is the Rust rewrite of the Bazzulto AArch64 kernel.
//! Boot path: QEMU virt → UEFI → Limine → _start (start.S) → kernel_main (here)

#![no_std]
#![no_main]

extern crate alloc;

mod arch;
mod display;
mod drivers;
mod fs;
mod hal;
mod ipc;
mod limine;
mod loader;
mod memory;
mod permission;
mod platform;
mod process;
mod scheduler;
mod smp;
mod sync;
mod systemcalls;
mod vdso;

use memory::heap::KernelGlobalAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: KernelGlobalAllocator = KernelGlobalAllocator;

use drivers::{console, uart};

// ---------------------------------------------------------------------------
// Limine requests
//
// Placed in .limine_requests so the bootloader can locate them.
// #[used] prevents the compiler from discarding them as dead code.
// ---------------------------------------------------------------------------

#[used]
#[link_section = ".limine_requests_start"]
static LIMINE_REQUESTS_START: [u64; 4] = [
    0xf6b8f4b39de7d1ae,
    0xfab91a6940fcb9cf,
    0x785c6ed015d3e316,
    0x181e920a7852b9d9,
];

#[used]
#[link_section = ".limine_requests_end"]
static LIMINE_REQUESTS_END: [u64; 2] = [0xadc0e0531bb10d03, 0x9572709f31764c62];

// Limine request objects MUST be in a writable section.
// Limine writes the response pointer into the `response` field at boot time.
// Using `static mut` ensures the compiler emits a writable section (W flag in ELF).
// All accesses are wrapped in `unsafe`; this is safe because Limine writes once
// before handing control to the kernel, and the kernel is single-core at this point.

#[used]
#[link_section = ".limine_requests"]
static mut FRAMEBUFFER_REQUEST: limine::FramebufferRequest = limine::FramebufferRequest {
    id: limine::FRAMEBUFFER_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut HHDM_REQUEST: limine::HhdmRequest = limine::HhdmRequest {
    id: limine::HHDM_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut MEMMAP_REQUEST: limine::MemmapRequest = limine::MemmapRequest {
    id: limine::MEMMAP_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut KERNEL_ADDRESS_REQUEST: limine::KernelAddressRequest = limine::KernelAddressRequest {
    id: limine::KERNEL_ADDRESS_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut BOOTLOADER_INFO_REQUEST: limine::BootloaderInfoRequest =
    limine::BootloaderInfoRequest {
        id: limine::BOOTLOADER_INFO_REQUEST_ID,
        revision: 0,
        response: core::ptr::null_mut(),
    };

#[used]
#[link_section = ".limine_requests"]
static mut DEVICE_TREE_BLOB_REQUEST: limine::DeviceTreeBlobRequest =
    limine::DeviceTreeBlobRequest {
        id: limine::DEVICE_TREE_BLOB_REQUEST_ID,
        revision: 0,
        response: core::ptr::null_mut(),
    };

#[used]
#[link_section = ".limine_requests"]
static mut SMP_REQUEST: limine::SmpRequest = limine::SmpRequest {
    id: limine::SMP_REQUEST_ID,
    revision: 0,
    flags: 0,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut KERNEL_FILE_REQUEST: limine::KernelFileRequest = limine::KernelFileRequest {
    id: limine::KERNEL_FILE_REQUEST_ID,
    revision: 0,
    response: core::ptr::null_mut(),
};

// ---------------------------------------------------------------------------
// Panic handler — no unwinding in a freestanding kernel
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write as _;

    // ── UART — always available ───────────────────────────────────────────────
    uart::puts("\r\nKERNEL PANIC: ");
    if let Some(msg) = info.message().as_str() {
        uart::puts(msg);
    } else {
        uart::puts("<format-args>");
    }
    if let Some(loc) = info.location() {
        uart::puts(" @ ");
        uart::puts(loc.file());
        uart::puts(":");
        let line = loc.line();
        let mut buf = [0u8; 10];
        let mut pos = 10usize;
        let mut n = line;
        if n == 0 {
            pos -= 1;
            buf[pos] = b'0';
        } else {
            while n > 0 {
                pos -= 1;
                buf[pos] = b'0' + (n % 10) as u8;
                n /= 10;
            }
        }
        if let Ok(s) = core::str::from_utf8(&buf[pos..]) {
            uart::puts(s);
        }
    }
    uart::puts("\r\n");

    // ── Framebuffer — so the crash is visible without a serial terminal ───────
    // Build a single-line header ("KERNEL PANIC: <message> @ file:line").
    // We need it as a &str, so format into a small stack buffer.
    let mut header_buf = [0u8; 256];
    let header_len = {
        let mut cursor = 0usize;
        let prefix = b"Rust panic: ";
        let copy_len = prefix.len().min(header_buf.len() - cursor);
        header_buf[cursor..cursor + copy_len].copy_from_slice(&prefix[..copy_len]);
        cursor += copy_len;
        if let Some(msg) = info.message().as_str() {
            let bytes = msg.as_bytes();
            let copy_len = bytes.len().min(header_buf.len() - cursor);
            header_buf[cursor..cursor + copy_len].copy_from_slice(&bytes[..copy_len]);
            cursor += copy_len;
        }
        if let Some(loc) = info.location() {
            let at = b" @ ";
            let copy_len = at.len().min(header_buf.len() - cursor);
            header_buf[cursor..cursor + copy_len].copy_from_slice(&at[..copy_len]);
            cursor += copy_len;
            let file = loc.file().as_bytes();
            let copy_len = file.len().min(header_buf.len().saturating_sub(cursor + 12));
            header_buf[cursor..cursor + copy_len].copy_from_slice(&file[..copy_len]);
            cursor += copy_len;
        }
        cursor
    };
    let header = core::str::from_utf8(&header_buf[..header_len]).unwrap_or("Rust panic");
    console::console_panic_screen(header);

    halt()
}

fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Boot cmdline helpers
// ---------------------------------------------------------------------------

/// Extract the value of a key from a kernel command-line string.
///
/// Scans `cmdline` for the first token of the form `key=value` and returns
/// a byte-slice of the value (up to the next space or end of string).
///
/// # Safety
/// `cmdline` must point to a valid null-terminated C string.
unsafe fn cmdline_get_value<'a>(cmdline: *const u8, key: &[u8]) -> Option<&'a [u8]> {
    if cmdline.is_null() {
        return None;
    }
    // Build a &[u8] slice of the entire cmdline (no allocation).
    let mut len = 0usize;
    while *cmdline.add(len) != 0 {
        len += 1;
    }
    let bytes = core::slice::from_raw_parts(cmdline, len);

    let mut i = 0usize;
    while i < bytes.len() {
        // Find end of current token (space or end).
        let token_end = bytes[i..].iter().position(|&b| b == b' ')
            .map(|pos| i + pos)
            .unwrap_or(bytes.len());
        let token = &bytes[i..token_end];

        // Check if token starts with "key=".
        if token.len() > key.len() + 1
            && token[..key.len()].eq_ignore_ascii_case(key)
            && token[key.len()] == b'='
        {
            return Some(&token[key.len() + 1..]);
        }

        i = token_end + 1; // skip past the space
    }
    None
}

/// Compare a FAT32 volume label (11-byte space-padded) against a name string.
///
/// The comparison is case-insensitive and ignores trailing spaces in the label.
fn volume_label_matches(label: &[u8; 11], name: &[u8]) -> bool {
    let trimmed = label.iter().rposition(|&b| b != b' ')
        .map(|pos| &label[..=pos])
        .unwrap_or(&label[..0]);
    trimmed.eq_ignore_ascii_case(name)
}

/// Parse a `root=UUID=XXXX-XXXX` value into a FAT32 Volume Serial Number.
///
/// Accepts 8 uppercase hex digits with an optional '-' separator in the
/// middle (e.g. `UUID=BAZ7-0001` or `UUID=BAZ70001`).
/// Returns `None` if the value does not start with `UUID=` or is not valid hex.
fn parse_uuid_root(value: &[u8]) -> Option<u32> {
    let prefix = b"UUID=";
    if value.len() < prefix.len() || !value[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    let hex_part = &value[prefix.len()..];
    // Accept "XXXX-XXXX" (9 chars) or "XXXXXXXX" (8 chars).
    let hex_str = core::str::from_utf8(hex_part).ok()?;
    let hex_clean: &str;
    let buf: [u8; 8];
    if hex_str.len() == 9 && hex_str.as_bytes()[4] == b'-' {
        // Rebuild without the dash into a stack buffer.
        buf = [
            hex_str.as_bytes()[0], hex_str.as_bytes()[1],
            hex_str.as_bytes()[2], hex_str.as_bytes()[3],
            hex_str.as_bytes()[5], hex_str.as_bytes()[6],
            hex_str.as_bytes()[7], hex_str.as_bytes()[8],
        ];
        hex_clean = core::str::from_utf8(&buf).ok()?;
    } else if hex_str.len() == 8 {
        hex_clean = hex_str;
    } else {
        return None;
    }
    u32::from_str_radix(hex_clean, 16).ok()
}

// kernel_main — called from start.S via `bl kernel_main`
// ---------------------------------------------------------------------------

/// Main kernel entry point.
///
/// Called by `_start` in `src/arch/arm64/boot/start.S` after the stack is set up.
/// `extern "C"` ensures the symbol name is not mangled and the calling convention
/// matches what the assembler `bl kernel_main` instruction expects (AAPCS64).
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // --- Step 0: get HHDM offset (needed by UART and console) ---
    let hhdm_offset = unsafe {
        if HHDM_REQUEST.response.is_null() {
            halt();
        }
        (*HHDM_REQUEST.response).offset
    };

    // --- Step 1: initialise early UART output ---
    uart::early_init(hhdm_offset);
    uart::puts("Bazzulto OS (Rust) booting...\r\n");

    // --- Step 2: initialise framebuffer console ---
    let fb = unsafe {
        if FRAMEBUFFER_REQUEST.response.is_null() {
            uart::puts("FATAL: no framebuffer response\r\n");
            halt();
        }
        let resp = &*FRAMEBUFFER_REQUEST.response;
        if resp.framebuffer_count < 1 {
            uart::puts("FATAL: no framebuffers\r\n");
            halt();
        }
        &**resp.framebuffers
    };

    // Initialise the framebuffer console before the resolution check so that
    // fatal errors are visible on screen, not only on the serial port.
    unsafe {
        console::init(fb.address, fb.width, fb.height, fb.pitch);
    }

    // Check minimum supported display resolution.
    // Production hardware requirement: 1024×768 minimum.
    // QEMU with ramfb + EDK2 AArch64 is limited to 800×600 — treated as a
    // warning in development; real hardware must meet the 1024×768 requirement.
    if fb.width < 1024 || fb.height < 768 {
        uart::puts("WARNING: display resolution ");
        uart::put_dec(fb.width as u64);
        uart::puts("x");
        uart::put_dec(fb.height as u64);
        uart::puts(" is below the recommended minimum 1024x768\r\n");
        console::print_str("WARNING: display ");
        console::print_dec(fb.width as u64);
        console::print_str("x");
        console::print_dec(fb.height as u64);
        console::print_str(" < 1024x768 minimum\n");
    }

    // Store framebuffer info for sys_framebuffer_map (userspace display server).
    unsafe {
        display::store(
            fb.address,
            fb.width, fb.height, fb.pitch,
            fb.bpp,
            fb.red_mask_size,   fb.red_mask_shift,
            fb.green_mask_size, fb.green_mask_shift,
            fb.blue_mask_size,  fb.blue_mask_shift,
            hhdm_offset,
        );
    }

    console::println("Bazzulto OS (Rust) booting...");

    // --- Step 3: print bootloader info ---
    unsafe {
        if !BOOTLOADER_INFO_REQUEST.response.is_null() {
            let resp = &*BOOTLOADER_INFO_REQUEST.response;
            console::print_str("Bootloader: ");
            console::print_str(cstr_to_str(resp.name));
            console::print_str(" ");
            console::println(cstr_to_str(resp.version));
        }
    }

    // --- Step 4: print HHDM info ---
    {
        use core::fmt::Write;
        let _ = writeln!(console::ConsoleWriter, "HHDM offset: {:#018x}", hhdm_offset);
    }

    // --- Step 5: count usable physical memory ---
    let usable_pages = unsafe {
        if MEMMAP_REQUEST.response.is_null() {
            uart::puts("FATAL: no memory map\r\n");
            halt();
        }
        let resp = &*MEMMAP_REQUEST.response;
        let mut pages: u64 = 0;
        for i in 0..resp.entry_count as usize {
            let entry = &**resp.entries.add(i);
            if entry.entry_type == limine::MEMMAP_USABLE {
                pages += entry.length / 4096;
            }
        }
        pages
    };

    {
        use core::fmt::Write;
        let _ = writeln!(
            console::ConsoleWriter,
            "Physical memory: {} pages usable",
            usable_pages
        );
    }

    // --- Step 6: print kernel load address ---
    unsafe {
        if !KERNEL_ADDRESS_REQUEST.response.is_null() {
            let resp = &*KERNEL_ADDRESS_REQUEST.response;
            use core::fmt::Write;
            let _ = writeln!(
                console::ConsoleWriter,
                "Kernel: phys {:#018x}  virt {:#018x}",
                resp.physical_base, resp.virtual_base
            );
        }
    }

    console::println("Phase 1 complete — framebuffer + UART operational.");
    uart::puts("Phase 1 complete.\r\n");

    // -----------------------------------------------------------------------
    // Phase 2: memory subsystem
    // -----------------------------------------------------------------------

    // Kernel address: needed to map the kernel image into our own page table.
    let (kernel_phys_base, kernel_virt_base) = unsafe {
        if KERNEL_ADDRESS_REQUEST.response.is_null() {
            uart::puts("FATAL: no kernel address response\r\n");
            halt();
        }
        let resp = &*KERNEL_ADDRESS_REQUEST.response;
        (resp.physical_base, resp.virtual_base)
    };

    let memmap_resp = unsafe {
        if MEMMAP_REQUEST.response.is_null() {
            uart::puts("FATAL: no memmap response\r\n");
            halt();
        }
        &*MEMMAP_REQUEST.response
    };

    unsafe {
        match memory::memory_init(
            hhdm_offset,
            memmap_resp,
            kernel_phys_base,
            kernel_virt_base,
        ) {
            Ok(()) => {}
            Err(e) => {
                uart::puts("FATAL: memory_init failed: ");
                uart::puts(match e {
                    memory::MapError::OutOfPhysicalMemory => "out of physical memory\r\n",
                    memory::MapError::UnalignedAddress    => "unaligned address\r\n",
                });
                halt();
            }
        }
    }

    {
        use core::fmt::Write;
        let _ = writeln!(console::ConsoleWriter, "Physical memory initialised.");
        let _ = writeln!(console::ConsoleWriter, "Virtual memory: kernel page table active.");
        let _ = writeln!(console::ConsoleWriter, "Heap: operational.");
        let _ = writeln!(console::ConsoleWriter, "Phase 2 complete.");
    }
    uart::puts("Phase 2 complete.\r\n");

    // -----------------------------------------------------------------------
    // Phase 3: exceptions + GIC + timer
    // -----------------------------------------------------------------------

    // Retrieve the DTB physical address from the Limine response (if provided).
    // Limine maps the DTB into its own address space; we pass the virtual address
    // directly since Limine runs in the same HHDM-mapped address space as the kernel.
    // The address is treated as a physical address for the HHDM-offset calculation
    // inside platform_init.  If Limine did not provide a DTB, pass 0.
    let dtb_phys_addr: u64 = unsafe {
        if !DEVICE_TREE_BLOB_REQUEST.response.is_null() {
            let resp = &*DEVICE_TREE_BLOB_REQUEST.response;
            if !resp.address.is_null() {
                // The DTB address provided by Limine is a virtual address within the
                // HHDM.  Convert back to physical by subtracting hhdm_offset.
                let virt_addr = resp.address as u64;
                if virt_addr >= hhdm_offset {
                    virt_addr - hhdm_offset
                } else {
                    // Address is not in HHDM — use as-is (may be identity-mapped).
                    virt_addr
                }
            } else {
                0
            }
        } else {
            0
        }
    };

    unsafe {
        platform::qemu_virt::platform_init(hhdm_offset, dtb_phys_addr);
    }

    {
        use core::fmt::Write;
        let _ = writeln!(console::ConsoleWriter, "Phase 3 complete — GIC + timer + exceptions active.");
    }
    uart::puts("Phase 3 complete.\r\n");

    // -----------------------------------------------------------------------
    // Phase 3.5: SMP — bring up secondary (AP) cores
    // -----------------------------------------------------------------------

    unsafe {
        if !SMP_REQUEST.response.is_null() {
            let smp_response = &*SMP_REQUEST.response;
            let cpu_count = smp_response.cpu_count as usize;

            uart::puts("SMP: BSP MPIDR = ");
            uart::put_hex(smp_response.bsp_mpidr);
            uart::puts("\r\n");

            for cpu_index in 0..cpu_count {
                // SAFETY: Limine guarantees cpu_count pointers in the cpus array,
                // each pointing to a valid SmpCpuInfo.
                let cpu_info = &mut **smp_response.cpus.add(cpu_index);

                if cpu_info.mpidr == smp_response.bsp_mpidr {
                    // This is the BSP; initialise its per-CPU data now that we know cpu_id.
                    // The BSP always receives cpu_id 0 in our scheme.
                    smp::per_cpu_init(0);
                    continue;
                }

                // Assign a cpu_id: use processor_id (0-based, sequential on QEMU virt).
                // Store it in extra_argument so ap_entry can read it without a lookup.
                let cpu_id = cpu_info.processor_id as u64;
                cpu_info.extra_argument = cpu_id;

                uart::puts("SMP: waking AP cpu_id=");
                uart::put_hex(cpu_id);
                uart::puts("\r\n");

                // Write goto_address with a volatile store.
                // Limine polls this field with a load-acquire; writing it last
                // (after extra_argument) ensures the AP sees the correct cpu_id.
                // Reference: Limine protocol specification, limine_smp_info::goto_address.
                core::ptr::write_volatile(
                    &mut cpu_info.goto_address,
                    smp::ap_entry as *const (),
                );
            }

            // Wait for all AP cores to complete their initialisation.
            // Use a spin loop with a finite timeout to avoid hanging forever if
            // a core fails to start (e.g. firmware does not support SMP).
            let expected_ap_count = cpu_count.saturating_sub(1); // exclude BSP
            let mut timeout_iterations: u64 = 10_000_000;
            while smp::AP_ONLINE_COUNT.load(core::sync::atomic::Ordering::Acquire)
                < expected_ap_count
                && timeout_iterations > 0
            {
                core::arch::asm!("nop");
                timeout_iterations -= 1;
            }

            let online_count = smp::AP_ONLINE_COUNT.load(core::sync::atomic::Ordering::Acquire);
            uart::puts("SMP: ");
            uart::put_hex(online_count as u64);
            uart::puts(" AP(s) online\r\n");
        } else {
            // Limine did not provide an SMP response; boot in single-core mode.
            // Still initialise BSP per-CPU data so current_cpu() works.
            smp::per_cpu_init(0);
            uart::puts("SMP: no Limine SMP response — single-core mode\r\n");
        }
    }

    // -----------------------------------------------------------------------
    // Phase 4: scheduler + file system + user processes
    // -----------------------------------------------------------------------

    // 4a. Initialise the scheduler (creates the idle process).
    let scheduler_ready = unsafe { scheduler::scheduler_init() };
    if !scheduler_ready {
        uart::puts("FATAL: scheduler_init failed\r\n");
        halt();
    }
    uart::puts("Scheduler initialised.\r\n");

    // 4b. Initialise virtio devices (disk + keyboard).
    unsafe {
        hal::disk::init(hhdm_offset);
        hal::keyboard::init(hhdm_offset);
    }
    uart::puts("VirtIO devices initialised.\r\n");

    // 4c. Initialise VFS (tmpfs root + devfs at /dev).
    unsafe {
        fs::vfs_init();
    }
    uart::puts("VFS initialised.\r\n");

    // 4d. Initialise block devices and mount FAT32 partitions.
    //
    // The partition to mount as "/" is identified by the `root=` value from
    // the Limine kernel cmdline (e.g. `cmdline: root=bazzulto`).  The value
    // is compared case-insensitively against each partition's FAT32 volume
    // label (BPB bytes 43–53, space-padded).
    //
    // If `root=` is absent, the first FAT32 partition is mounted as "/" (same
    // behaviour as before).  Additional partitions are mounted at
    // /mnt/disk{letter}{N}.
    uart::puts("[storage] initializing...\r\n");
    {
        // Read root= from Limine kernel cmdline.
        let root_label: Option<&[u8]> = unsafe {
            let resp = KERNEL_FILE_REQUEST.response;
            if resp.is_null() {
                uart::puts("[storage] no kernel file response — root= not available\r\n");
                None
            } else {
                let file = (*resp).kernel_file;
                if file.is_null() {
                    None
                } else {
                    let cmdline = cmdline_get_value((*file).cmdline, b"root");
                    if let Some(label) = cmdline {
                        uart::puts("[storage] root= label: ");
                        uart::puts(core::str::from_utf8(label).unwrap_or("?"));
                        uart::puts("\r\n");
                    }
                    cmdline
                }
            }
        };

        let disk_count = hal::disk::disk_count();
        if disk_count == 0 {
            uart::puts("[storage] no block devices — running from ramfs only\r\n");
        }
        let mut root_mounted = false;
        for disk_index in 0..disk_count {
            let disk = match hal::disk::get_disk(disk_index) {
                Some(device) => device,
                None => continue,
            };
            let partitions = fs::partition::enumerate_partitions(disk, disk_index);
            for partition in partitions {
                // ── Btrfs probe ─────────────────────────────────────────────
                // Check for Btrfs superblock magic (_BHRfS_M at 64 KiB).
                // Btrfs is the default root filesystem from v1.0 onward.
                //
                // Root selection: match the Btrfs volume label against the
                // root= cmdline value (same as FAT32).  If root= is absent,
                // fall back to label "BAZZULTO".
                if fs::btrfs::btrfs_probe(&partition.disk, partition.start_lba) {
                    let btrfs_root = match fs::btrfs::btrfs_mount(
                        partition.disk.clone(), partition.start_lba
                    ) {
                        Some(root) => root,
                        None => {
                            uart::puts("[storage] Btrfs probe passed but mount failed — skipping\r\n");
                            continue;
                        }
                    };

                    // Determine if this Btrfs partition should be root.
                    //
                    // Match logic:
                    // - root=BAZZULTO   → match by label (case-insensitive)
                    // - root=UUID=...   → UUID is FAT32-specific, so fall back
                    //                     to default label match ("BAZZULTO")
                    // - root= absent    → match default label "BAZZULTO"
                    let btrfs_label = fs::btrfs::btrfs_label(&*partition.disk, partition.start_lba);
                    let is_root_candidate = if !root_mounted {
                        match &btrfs_label {
                            Some(vol_label) => {
                                let is_uuid_root = root_label.map_or(false, |l|
                                    l.len() > 5 && l[..5].eq_ignore_ascii_case(b"UUID="));
                                if is_uuid_root || root_label.is_none() {
                                    // UUID= is FAT32-specific or no root= at all:
                                    // default to matching label "BAZZULTO".
                                    vol_label == "BAZZULTO"
                                } else if let Some(label) = root_label {
                                    // root= is a plain label: match directly.
                                    vol_label.as_bytes().eq_ignore_ascii_case(label)
                                } else {
                                    false
                                }
                            }
                            None => false,
                        }
                    } else {
                        false
                    };

                    if is_root_candidate {
                        unsafe { fs::vfs_mount("/", btrfs_root, &partition.device_path(), "btrfs"); }
                        uart::puts("[storage] Btrfs mounted as / (label: ");
                        if let Some(ref l) = btrfs_label {
                            uart::puts(l);
                        }
                        uart::puts(")\r\n");
                        root_mounted = true;
                    } else {
                        // Non-root Btrfs: defer until root is mounted.
                        uart::puts("[storage] Btrfs partition deferred (label: ");
                        if let Some(ref l) = btrfs_label {
                            uart::puts(l);
                        }
                        uart::puts(")\r\n");
                    }
                    continue;
                }

                // ── BAFS probe ──────────────────────────────────────────────
                // Check for BAFS magic before FAT32.  Any partition type may
                // carry a BAFS filesystem (BAFS does not have a reserved MBR
                // type byte yet).  The probe reads one sector and is cheap.
                if fs::bafs_driver::bafs_probe(&partition.disk, partition.start_lba) {
                    let bafs_root = match fs::bafs_driver::bafs_mount_partition(
                        partition.disk.clone(), partition.start_lba
                    ) {
                        Some(root) => root,
                        None => {
                            uart::puts("[storage] BAFS probe passed but mount failed — skipping\r\n");
                            continue;
                        }
                    };
                    let mount_path = partition.mount_path();
                    unsafe {
                        let dir_name = mount_path.trim_start_matches("/mnt/");
                        if let Ok((mnt_inode, _)) = fs::vfs_resolve_parent("/mnt/placeholder") {
                            let _ = mnt_inode.mkdir(dir_name);
                        }
                        fs::vfs_mount(&mount_path, bafs_root, &partition.device_path(), "bafs");
                    }
                    uart::puts("[storage] BAFS mounted at ");
                    uart::puts(&mount_path);
                    uart::puts("\r\n");
                    continue;
                }

                // ── FAT32 probe ─────────────────────────────────────────────
                if !partition.is_fat32_candidate() { continue; }
                let fat32_volume = match fs::fat32::fat32_init_partition(
                    partition.disk.clone(), partition.start_lba
                ) {
                    Some(vol) => vol,
                    None => continue,
                };
                let Some(fat32_root) = fs::fat32::fat32_root_inode(fat32_volume.clone()) else { continue };

                // Decide whether this partition should become the root mount.
                // - If root=UUID=XXXX-XXXX: match by FAT32 Volume Serial Number.
                // - If root=LABEL:          match by volume label (fallback for
                //                           development without a fixed UUID).
                // - If root= absent:        mount the first FAT32/Btrfs found as root.
                let is_root_candidate = match root_label {
                    Some(label) => {
                        if let Some(target_uuid) = parse_uuid_root(label) {
                            let vol_uuid = fs::fat32::fat32_volume_uuid(&fat32_volume);
                            vol_uuid == target_uuid
                        } else {
                            let vol = fs::fat32::fat32_volume_label(&fat32_volume);
                            volume_label_matches(&vol, label)
                        }
                    }
                    None => !root_mounted,
                };

                if is_root_candidate && !root_mounted {
                    // Mount as the root filesystem.
                    // The root partition is always named //dev:diska:1/ by convention —
                    // it is the canonical "first data disk" regardless of which physical
                    // virtio-mmio slot it occupies (that slot depends on firmware/QEMU
                    // enumeration order, not on OS-level disk identity).
                    unsafe { fs::vfs_mount("/", fat32_root, "//dev:diska:1/", "fat32"); }
                    uart::puts("[storage] FAT32 mounted as /\r\n");
                    root_mounted = true;
                } else {
                    // Additional partition — deferred to bzinit via
                    // /system/config/disk-mounts.  Do not mount here.
                    uart::puts("[storage] additional partition found, deferred to bzinit\r\n");
                }
            }
        }
        if !root_mounted {
            if root_label.is_some() {
                // A specific root= was requested but not found — this is fatal.
                // The system cannot boot without a root filesystem.
                uart::puts("[storage] FATAL: root partition not found — kernel cannot continue\r\n");
                uart::puts("[storage] Check that root= in limine.conf matches the disk UUID or label.\r\n");
                loop {
                    core::hint::spin_loop();
                }
            }
            uart::puts("[storage] WARNING: no Btrfs/FAT32 disk found — /system/bin/bzinit must be in ramfs\r\n");
        }

        // Second pass: mount deferred Btrfs partitions now that root is mounted.
        // The first non-root Btrfs volume is mounted at /home/user.
        if root_mounted {
            let mut home_mounted = false;
            for disk_index in 0..disk_count {
                if home_mounted { break; }
                let disk = match hal::disk::get_disk(disk_index) {
                    Some(device) => device,
                    None => continue,
                };
                let partitions = fs::partition::enumerate_partitions(disk, disk_index);
                for partition in partitions {
                    if home_mounted { break; }
                    if !fs::btrfs::btrfs_probe(&partition.disk, partition.start_lba) {
                        continue;
                    }
                    let btrfs_label = fs::btrfs::btrfs_label(&*partition.disk, partition.start_lba);
                    let is_root = btrfs_label.as_deref() == Some("BAZZULTO");
                    if is_root { continue; } // Skip the root partition.

                    let btrfs_root = match fs::btrfs::btrfs_mount(
                        partition.disk.clone(), partition.start_lba
                    ) {
                        Some(root) => root,
                        None => continue,
                    };
                    unsafe {
                        fs::vfs_mount("/home/user", btrfs_root, &partition.device_path(), "btrfs");
                    }
                    uart::puts("[storage] Btrfs mounted at /home/user (label: ");
                    if let Some(ref l) = btrfs_label {
                        uart::puts(l);
                    }
                    uart::puts(")\r\n");
                    home_mounted = true;
                }
            }
        }
    }
    uart::puts("[storage] ready\r\n");

    // 4e. Spawn bzinit as PID 1 from the VFS (FAT32 disk).
    //
    // All userspace binaries now live on the disk image rather than being
    // embedded in the kernel binary.  The kernel simply reads bzinit from
    // /system/bin/bzinit and launches it; bzinit then starts all services.
    unsafe {
        // Mark bzinit as kernel-exec-only before spawning it.
        // This ensures no userspace process can re-exec bzinit after boot.
        // Reference: docs/features/Binary Permission Model.md §INODE_KERNEL_EXEC_ONLY.
        fs::vfs_mark_kernel_exec_only("/system/bin/bzinit");

        // Spawn bzinit as PID 1.
        let spawn_result = scheduler::with_scheduler(|sched| {
            loader::spawn_from_vfs(sched, "/system/bin/bzinit")
        });
        match spawn_result {
            Ok(pid) => {
                // Register PID 1 with the scheduler for orphan reparenting.
                scheduler::with_scheduler(|sched| sched.set_init_pid(pid));

                // Grant bzinit all capabilities and root identity.
                // PID 1 is the system init — it runs as uid=0 ("system")
                // and distributes narrower identity/permissions to children.
                scheduler::with_scheduler(|sched| {
                    if let Some(process) = sched.process_mut(pid) {
                        process.capabilities = crate::process::CAP_ALL;
                        process.uid  = 0;
                        process.gid  = 0;
                        process.euid = 0;
                        process.egid = 0;
                        process.suid = 0;
                        process.sgid = 0;
                    }
                });

                // Set initial working directory to VFS root.
                let root_inode = fs::vfs_resolve("/", None).ok();
                if let Some(inode) = root_inode {
                    scheduler::with_scheduler(|sched| {
                        if let Some(process) = sched.process_mut(pid) {
                            process.cwd = Some(inode);
                        }
                    });
                }

                uart::puts("Spawned bzinit as PID ");
                uart::put_hex(pid.index as u64);
                uart::puts(" (PID 1 — init)\r\n");
            }
            Err(_) => {
                uart::puts("FATAL: failed to spawn bzinit — system cannot boot\r\n");
                crate::drivers::console::console_panic_screen(
                    "Failed to spawn /system/bin/bzinit — system cannot boot.\n\
                     \n\
                     The root filesystem was mounted but bzinit could not be\n\
                     loaded.  Verify that the disk image contains a valid ELF\n\
                     at /system/bin/bzinit."
                );
                loop { core::hint::spin_loop(); }
            }
        }
    }

    // 4e. Enable TTBR0 walks (required before any eret to EL0).
    memory::virtual_memory::PageTable::enable_user_space();

    {
        use core::fmt::Write;
        let _ = writeln!(console::ConsoleWriter, "Phase 4 complete — scheduler active.");
    }
    uart::puts("Phase 4 complete.\r\n");

    // 4f. Hand off to the scheduler.  The idle process runs wfe in a loop.
    // The timer IRQ (Phase 3) will preempt when the first user process is ready.
    //
    // `schedule_next()` acquires the scheduler spinlock, calls schedule()
    // (which may context-switch to another process), and releases the lock.
    // Using schedule_next() rather than with_scheduler + schedule() is
    // necessary because schedule() releases the spinlock before context_switch()
    // and reacquires it after; the outer with_scheduler would double-release.
    loop {
        unsafe {
            scheduler::schedule_next();
            core::arch::asm!("wfe", options(nomem, nostack));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a NUL-terminated C string pointer to a Rust &str.
///
/// # Safety
/// `ptr` must point to a valid NUL-terminated byte sequence.
unsafe fn cstr_to_str(ptr: *const u8) -> &'static str {
    if ptr.is_null() {
        return "<null>";
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
}
