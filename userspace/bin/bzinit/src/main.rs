//! bzinit — Bazzulto PID 1 service manager.
//!
//! Boot sequence:
//!   1. Validate POSIX compat translation targets
//!   2. Load service files from /config/bazzulto/services/*.service
//!   3. Topological sort by dependency
//!   4. Spawn services in order
//!   5. Wait loop: reap children, restart crashed services, write state

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

    // Step 1 — validate POSIX compat translation targets
    compat::validate_compat_targets();

    // Step 2 — load service files
    let mut services = load_service_files();

    if services.is_empty() {
        let _ = error_output.write_line("bzinit: warn: no service files found");
    }

    // Step 3 — topological sort
    let order = topological_order(&services);

    // Step 3b — create the display pipe so bzdisplayd can receive output from
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
    let state_fd_result = raw::raw_creat(state_path.as_ptr(), state_path.len());
    let state_fd: i32 = if state_fd_result >= 0 { state_fd_result as i32 } else { -1 };

    // Step 6 — wait loop
    loop {
        let mut exit_status: i32 = 0;
        let child_pid = raw::raw_wait(-1, &mut exit_status as *mut i32);

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
// Service file loading
// ---------------------------------------------------------------------------

/// Enumerate /config/bazzulto/services/*.service via getdents64 and parse each.
fn load_service_files() -> Vec<ServiceState> {
    let mut states = Vec::new();

    let service_entries = Directory::list_with_suffix_in("/config/bazzulto/services/", ".service");
    for entry_name in service_entries {

        match File::open(&entry_name) {
            Ok(file) => {
                match file.read_to_end() {
                    Ok(content) => {
                        if let Some(definition) = ServiceDefinition::from_bytes(&content) {
                            states.push(ServiceState::new(definition));
                        }
                    }
                    Err(_) => {}
                }
            }
            Err(_) => {}
        }
    }

    states
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
