//! bzinit — Bazzulto PID 1 service manager.
//!
//! Boot sequence:
//!   1. Process /system/config/disk-mounts — mount additional disks
//!   2. Validate POSIX compat translation targets
//!   3. Load service files from /system/config/services/*.service
//!   4. Topological sort by dependency
//!   5. Spawn services in order
//!   6. Wait loop: reap children, restart crashed services, write state

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

mod compat;
mod dependency;
mod service;
mod supervisor;
mod toml_parser;

use alloc::vec::Vec;
use bazzulto_system::raw;
use bazzulto_io::directory::Directory;
use bazzulto_io::file::File;
use bazzulto_io::stream::{stdout, stderr};
use service::{ServiceDefinition, ServiceState};
use supervisor::{boot_services, handle_child_exit, write_state, DisplayPipe};
use dependency::topological_order;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    main();
    raw::raw_exit(0)
}

fn main() {
    let standard_output = stdout();
    let error_output = stderr();

    let _ = standard_output.write_line("bzinit: starting");

    // Step 1 — mount additional disks listed in /system/config/disk-mounts.
    // The root filesystem is already mounted by the kernel at this point.
    process_disk_mounts("/system/config/disk-mounts");

    // Diagnostic: list key directories so boot problems are visible in logs.
    log_directory_contents(&standard_output, "/");
    log_directory_contents(&standard_output, "/data/test/");
    log_directory_contents(&standard_output, "/system/config/");
    log_directory_contents(&standard_output, "/system/config/services/");

    // Step 2 — validate POSIX compat translation targets
    compat::validate_compat_targets();

    // Step 3 — load service files
    let mut services = load_service_files();

    if services.is_empty() {
        let _ = error_output.write_line("bzinit: warn: no service files found");
    }

    // Step 4 — topological sort
    let order = topological_order(&services);

    // Step 4b — create the display pipe so bzdisplayd can receive output from
    // other services.  Failure here is non-fatal: services fall back to the
    // kernel console.
    let display_pipe = DisplayPipe::create();
    if display_pipe.is_none() {
        let _ = error_output.write_line(
            "bzinit: warn: could not create display pipe — output stays on kernel console",
        );
    }

    // Step 4 — spawn services in dependency order
    boot_services(&mut services, &order, display_pipe.as_ref());

    // Step 5 — open the state fd (if supported by kernel; ignore error)
    let state_path = "/proc/bzinit/state";
    let mut state_path_buf = [0u8; 512];
    let state_path_len = state_path.len().min(511);
    state_path_buf[..state_path_len].copy_from_slice(&state_path.as_bytes()[..state_path_len]);
    let state_fd_result = raw::raw_creat(state_path_buf.as_ptr(), 0o600);
    let state_fd: i32 = if state_fd_result >= 0 { state_fd_result as i32 } else { -1 };

    // Step 6 — wait loop
    loop {
        let mut exit_status: i32 = 0;
        let child_pid = raw::raw_wait(-1, &mut exit_status as *mut i32, 0);

        if child_pid > 0 {
            handle_child_exit(&mut services, child_pid as i32, exit_status, display_pipe.as_ref());
        }

        // Write state snapshot
        if state_fd >= 0 {
            // Rewind to start of file before writing.
            raw::raw_seek(state_fd, 0, 0 /* SEEK_SET */);
            write_state(&services, state_fd);
        }

        raw::raw_yield();
    }
}

// ---------------------------------------------------------------------------
// Diagnostic logging
// ---------------------------------------------------------------------------

/// Log all entries visible in `dir_path` to stdout.
///
/// Prints nothing if the directory cannot be opened (non-fatal).
fn log_directory_contents(output: &bazzulto_io::stream::Stream, dir_path: &str) {
    use bazzulto_io::directory::Directory;
    let entries = Directory::read_dir(dir_path);
    if entries.is_empty() {
        let _ = output.write_all(b"bzinit: ls ");
        let _ = output.write_all(dir_path.as_bytes());
        let _ = output.write_all(b" -> (empty or not found)\n");
    } else {
        let _ = output.write_all(b"bzinit: ls ");
        let _ = output.write_all(dir_path.as_bytes());
        let _ = output.write_all(b" ->\n");
        for entry in &entries {
            let _ = output.write_all(b"bzinit:   ");
            let _ = output.write_all(entry.as_bytes());
            let _ = output.write_all(b"\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Service file loading
// ---------------------------------------------------------------------------

/// Enumerate /system/config/services/*.service via getdents64 and parse each.
fn load_service_files() -> Vec<ServiceState> {
    let error_output = stderr();
    let mut states = Vec::new();

    let service_entries = Directory::list_with_suffix_in("/system/config/services/", ".service");
    for entry_name in service_entries {
        match File::open(&entry_name) {
            Ok(file) => {
                match file.read_to_end() {
                    Ok(content) => {
                        if let Some(definition) = ServiceDefinition::from_bytes(&content) {
                            let _ = error_output.write_all(b"bzinit: loaded service: ");
                            let _ = error_output.write_all(definition.name.as_bytes());
                            let _ = error_output.write_all(b"\n");
                            states.push(ServiceState::new(definition));
                        } else {
                            let _ = error_output.write_all(b"bzinit: warn: failed to parse service file: ");
                            let _ = error_output.write_all(entry_name.as_bytes());
                            let _ = error_output.write_all(b"\n");
                        }
                    }
                    Err(_) => {
                        let _ = error_output.write_all(b"bzinit: warn: failed to read service file: ");
                        let _ = error_output.write_all(entry_name.as_bytes());
                        let _ = error_output.write_all(b"\n");
                    }
                }
            }
            Err(_) => {
                let _ = error_output.write_all(b"bzinit: warn: failed to open service file: ");
                let _ = error_output.write_all(entry_name.as_bytes());
                let _ = error_output.write_all(b"\n");
            }
        }
    }

    states
}

// ---------------------------------------------------------------------------
// process_disk_mounts — read /system/config/disk-mounts and mount each entry
// ---------------------------------------------------------------------------

/// Read the disk-mounts configuration file and mount each listed filesystem.
///
/// File format:
///   # comment lines are ignored
///   //dev:diskb:1/    //home:user/    fat32
///
/// Columns (whitespace-separated):
///   1. device     — Bazzulto Path Model device path
///   2. mountpoint — Bazzulto Path Model or POSIX path
///   3. filesystem — "fat32", "bafs", or "tmpfs"
///
/// Failures to mount individual entries are logged to stderr but do not
/// stop processing of remaining entries.
fn process_disk_mounts(path: &str) {
    let error_output = stderr();
    let file = match File::open(path) {
        Ok(f)  => f,
        Err(_) => return, // file absent — no additional disks to mount
    };
    let content = match file.read_to_string() {
        Ok(c)  => c,
        Err(_) => return,
    };
    for line in content.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_ascii_whitespace();
        let (device, mountpoint, fstype) = match (fields.next(), fields.next(), fields.next()) {
            (Some(d), Some(m), Some(f)) => (d, m, f),
            _ => continue,
        };
        // Create the mountpoint directory if it does not exist.
        bazzulto_io::create_dir_all(mountpoint);
        // Mount the filesystem.
        match bazzulto_io::mount(device, mountpoint, fstype) {
            Ok(_)  => {
                let _ = error_output.write_all(b"bzinit: mounted ");
                let _ = error_output.write_all(device.as_bytes());
                let _ = error_output.write_all(b" -> ");
                let _ = error_output.write_all(mountpoint.as_bytes());
                let _ = error_output.write_all(b"\n");
            }
            Err(e) => {
                let _ = error_output.write_all(b"bzinit: disk-mounts: failed to mount ");
                let _ = error_output.write_all(device.as_bytes());
                let _ = error_output.write_all(b" at ");
                let _ = error_output.write_all(mountpoint.as_bytes());
                let _ = error_output.write_all(b" errno=");
                // Simple u32 → decimal conversion without alloc.
                let mut digits = [0u8; 10];
                let mut n = e as u32;
                let mut pos = 10usize;
                if n == 0 { pos -= 1; digits[pos] = b'0'; }
                while n > 0 { pos -= 1; digits[pos] = b'0' + (n % 10) as u8; n /= 10; }
                let _ = error_output.write_all(&digits[pos..]);
                let _ = error_output.write_all(b"\n");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    let error_output = stderr();
    let _ = error_output.write_all(b"bzinit: panic\n");
    raw::raw_exit(1)
}
