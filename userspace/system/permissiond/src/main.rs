// permissiond — Bazzulto Binary Permission Model daemon.
//
// Completely standalone — no BSL, no alloc, no crate dependencies.
// All syscalls via inline SVC to eliminate any initialization issues.

#![no_std]
#![no_main]
extern crate coreutils;

// ---------------------------------------------------------------------------
// Raw syscall wrappers — inline SVC only, no vDSO, no BSL.
// ---------------------------------------------------------------------------

fn sys_write(fd: i32, ptr: *const u8, len: usize) -> i64 {
    let r: i64;
    unsafe {
        core::arch::asm!("svc #1",
            in("x0") fd as u64, in("x1") ptr as u64, in("x2") len as u64,
            lateout("x0") r, options(nostack));
    }
    r
}

fn sys_read(fd: i32, ptr: *mut u8, len: usize) -> i64 {
    let r: i64;
    unsafe {
        core::arch::asm!("svc #2",
            in("x0") fd as u64, in("x1") ptr as u64, in("x2") len as u64,
            lateout("x0") r, options(nostack));
    }
    r
}

fn sys_yield() {
    unsafe { core::arch::asm!("svc #3", options(nostack)); }
}

fn sys_exit(code: i32) -> ! {
    unsafe { core::arch::asm!("svc #0", in("x0") code as u64, options(nostack, noreturn)); }
}

fn bpm_register() -> i64 {
    let r: i64;
    unsafe { core::arch::asm!("svc #167", lateout("x0") r, options(nostack)); }
    r
}

fn bpm_read_request(buf: &mut [u8]) -> i64 {
    let r: i64;
    unsafe {
        core::arch::asm!("svc #168",
            in("x0") buf.as_mut_ptr() as u64, in("x1") buf.len() as u64,
            lateout("x0") r, options(nostack));
    }
    r
}

fn bpm_respond(blocked_pid: u32, decision: u32, patterns: &[u8]) -> i64 {
    let r: i64;
    unsafe {
        core::arch::asm!("svc #169",
            in("x0") blocked_pid as u64, in("x1") decision as u64,
            in("x2") patterns.as_ptr() as u64, in("x3") patterns.len() as u64,
            lateout("x0") r, options(nostack));
    }
    r
}

fn out(msg: &[u8]) {
    sys_write(2, msg.as_ptr(), msg.len());
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    // Initialize BSL runtime.
    bazzulto_system::init_with_args_envp(argc, argv, envp);

    // Step 1: Register with the kernel.
    let result = bpm_register();
    if result < 0 {
        out(b"permissiond: register failed\n");
        sys_exit(1);
    }

    // Step 2: Main loop — read requests, respond.
    let mut req_buf = [0u8; 512];
    loop {
        let n = bpm_read_request(&mut req_buf);
        if n <= 0 {
            sys_yield();
            continue;
        }

        // Parse minimal fields.
        if (n as usize) < 12 { continue; }
        let blocked_pid = u32::from_le_bytes([req_buf[0], req_buf[1], req_buf[2], req_buf[3]]);
        let has_tty = req_buf[9] != 0;

        if has_tty {
            // Parse binary path for prompt.
            let hash_len = u16::from_le_bytes([req_buf[10], req_buf[11]]) as usize;
            let path_len_off = 12 + hash_len;
            if path_len_off + 2 > n as usize { bpm_respond(blocked_pid, 2, &[]); continue; }
            let path_len = u16::from_le_bytes([req_buf[path_len_off], req_buf[path_len_off + 1]]) as usize;
            let path_off = path_len_off + 2;
            let path = if path_off + path_len <= n as usize {
                &req_buf[path_off..path_off + path_len]
            } else {
                b"?"
            };

            // Show interactive prompt on the display (fd 1 = display pipe).
            sys_write(1, b"\n[bazzulto] ".as_ptr(), 12);
            sys_write(1, path.as_ptr(), path.len());
            sys_write(1, b" has no permission record.\n".as_ptr(), 27);
            sys_write(1, b"           Allow execution? [yes/No]: ".as_ptr(), 38);

            // Read response from keyboard (fd 0 = TTY).
            // Accept "yes" (or "y") — anything else is denied.
            let mut input = [0u8; 64];
            let nr = sys_read(0, input.as_mut_ptr(), input.len());
            let response = if nr > 0 {
                let len = (nr as usize).min(input.len());
                // Strip trailing newline/CR.
                let mut end = len;
                while end > 0 && (input[end - 1] == b'\n' || input[end - 1] == b'\r') {
                    end -= 1;
                }
                &input[..end]
            } else {
                &[]
            };
            let approved = response == b"yes" || response == b"y";
            sys_write(1, b"\n".as_ptr(), 1);

            if approved {
                bpm_respond(blocked_pid, 2, &[]); // GrantedInherited
            } else {
                bpm_respond(blocked_pid, 0, &[]); // Denied
            }
        } else {
            // No TTY — auto-grant inherited permissions.
            bpm_respond(blocked_pid, 2, &[]);
        }
    }
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    sys_exit(1);
}
