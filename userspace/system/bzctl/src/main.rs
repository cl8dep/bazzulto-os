//! bzctl — Bazzulto service manager CLI.
//!
//! v1.0 commands:
//!   bzctl status          — print all service states
//!   bzctl status <name>   — print state of a single service

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::vec::Vec;
use bazzulto_system::raw;
use bazzulto_io::file::File;
use bazzulto_io::stream::{stdout, stderr};

const STATE_PATH: &str = "/proc/bzinit/state";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    // Argument parsing is not yet available in v1.0 — always print full status.
    cmd_status(None);
    raw::raw_exit(0)
}

// ---------------------------------------------------------------------------
// bzctl status [name]
// ---------------------------------------------------------------------------

fn cmd_status(filter: Option<&str>) {
    let standard_output = stdout();
    let error_output = stderr();

    let file = match File::open(STATE_PATH) {
        Ok(file) => file,
        Err(_) => {
            let _ = error_output.write_line("bzctl: cannot read /proc/bzinit/state (bzinit not running?)");
            return;
        }
    };

    let content = match file.read_to_string() {
        Ok(string_value) => string_value,
        Err(_) => {
            let _ = error_output.write_line("bzctl: state file is not valid UTF-8");
            return;
        }
    };

    // Print header.
    let _ = standard_output.write_line("NAME                STATUS     PID     RETRIES");
    let _ = standard_output.write_line("----                ------     ---     -------");

    let mut found_any = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Line format: "name status pid=PID retries=N"
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 4 {
            continue;
        }

        let service_name   = parts[0];
        let service_status = parts[1];
        let pid_part       = parts[2];  // "pid=PID"
        let retries_part   = parts[3];  // "retries=N"

        if let Some(ref name_filter) = filter {
            if service_name != *name_filter {
                continue;
            }
        }

        found_any = true;

        let pid_value     = pid_part.trim_start_matches("pid=");
        let retries_value = retries_part.trim_start_matches("retries=");

        // Pad columns for readability.
        let _ = standard_output.write_all(pad_right(service_name, 20).as_bytes());
        let _ = standard_output.write_all(pad_right(service_status, 11).as_bytes());
        let _ = standard_output.write_all(pad_right(pid_value, 8).as_bytes());
        let _ = standard_output.write_line(retries_value);
    }

    if !found_any {
        if let Some(name_filter) = filter {
            let _ = error_output.write_all(b"bzctl: service '");
            let _ = error_output.write_all(name_filter.as_bytes());
            let _ = error_output.write_line("' not found");
        } else {
            let _ = standard_output.write_line("(no services)");
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Right-pad `string_value` with spaces to `width`. Returns owned String.
fn pad_right(string_value: &str, width: usize) -> alloc::string::String {
    let mut result = alloc::string::String::from(string_value);
    while result.len() < width {
        result.push(' ');
    }
    result
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
