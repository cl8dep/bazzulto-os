//! Service supervisor — spawn, monitor, and restart services.

use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;
use bazzulto_system::raw;
use bazzulto_system::time::{Time, Duration, clock_gettime, ClockId};
use bazzulto_system::thread::Thread;
use bazzulto_io::stream::stderr;
use crate::service::{ServiceState, ServiceStatus, RestartStrategy};

// ---------------------------------------------------------------------------
// Display pipe
// ---------------------------------------------------------------------------

/// Holds the read and write ends of the display output pipe.
///
/// - `read_fd`  — bzdisplayd's stdin: it reads text from here and renders it.
/// - `write_fd` — other services' stdout/stderr: their output arrives here.
///
/// A value of -1 means the pipe could not be created (e.g. headless boot).
pub struct DisplayPipe {
    pub read_fd:  i32,
    pub write_fd: i32,
}

impl DisplayPipe {
    /// Create the pipe.  Returns `None` if the syscall fails.
    pub fn create() -> Option<DisplayPipe> {
        let mut fd_pair = [0i32; 2];
        let result = raw::raw_pipe(fd_pair.as_mut_ptr());
        if result < 0 {
            return None;
        }
        Some(DisplayPipe {
            read_fd:  fd_pair[0],
            write_fd: fd_pair[1],
        })
    }
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// Spawn services in the dependency-resolved order given by `order`.
///
/// `display_pipe` — if `Some`, used to wire display-server stdin and other
/// services' stdout/stderr through the pipe so all output is rendered by
/// bzdisplayd.
pub fn boot_services(
    services: &mut Vec<ServiceState>,
    order: &[usize],
    display_pipe: Option<&DisplayPipe>,
) {
    for &index in order {
        spawn_service(&mut services[index], display_pipe);
    }
}

// ---------------------------------------------------------------------------
// Crash recovery
// ---------------------------------------------------------------------------

/// Called when a child with the given `pid` and `exit_status` terminates.
/// Updates the matching service state and restarts it according to its policy.
pub fn handle_child_exit(
    services: &mut Vec<ServiceState>,
    pid: i32,
    exit_status: i32,
    display_pipe: Option<&DisplayPipe>,
) {
    let error_output = stderr();

    for service_state in services.iter_mut() {
        if service_state.pid != pid {
            continue;
        }

        service_state.pid = 0;

        match service_state.status {
            ServiceStatus::Stopped => {
                // Intentional stop — do not restart.
                return;
            }
            _ => {}
        }

        // Unexpected exit — apply restart strategy.
        service_state.status = ServiceStatus::Failed;

        match service_state.definition.restart_strategy {
            RestartStrategy::Never => {
                let _ = error_output.write_all(b"bzinit: service '");
                let _ = error_output.write_all(service_state.definition.name.as_bytes());
                let _ = error_output.write_all(b"' exited and will not be restarted\n");
            }

            RestartStrategy::Always => {
                let _ = error_output.write_all(b"bzinit: restarting '");
                let _ = error_output.write_all(service_state.definition.name.as_bytes());
                let _ = error_output.write_all(b"'\n");
                spawn_service(service_state, display_pipe);
            }

            RestartStrategy::ExponentialBackoff => {
                if service_state.retry_count >= service_state.definition.max_retries {
                    // Try fallback binary if set, else give up.
                    if let Some(ref fallback_binary) = service_state.definition.fallback.clone() {
                        let _ = error_output.write_all(b"bzinit: '");
                        let _ = error_output.write_all(service_state.definition.name.as_bytes());
                        let _ = error_output.write_all(b"' max retries reached, trying fallback\n");
                        spawn_with_binary(service_state, fallback_binary.clone(), display_pipe);
                    } else {
                        let _ = error_output.write_all(b"bzinit: '");
                        let _ = error_output.write_all(service_state.definition.name.as_bytes());
                        let _ = error_output.write_all(b"' max retries reached, giving up\n");
                        service_state.status = ServiceStatus::Failed;
                    }
                } else {
                    // Backoff: 2^retry_count seconds, capped at 64s.
                    let backoff_seconds = 1u64 << service_state.retry_count.min(6);
                    service_state.retry_count += 1;

                    let _ = error_output.write_all(b"bzinit: restarting '");
                    let _ = error_output.write_all(service_state.definition.name.as_bytes());
                    let _ = error_output.write_all(b"' (attempt ");
                    // Simple u32 to ASCII.
                    let retry_str = format_u32(service_state.retry_count);
                    let _ = error_output.write_all(retry_str.as_bytes());
                    let _ = error_output.write_all(b")\n");

                    let _ = Thread::sleep(Duration::secs(backoff_seconds));
                    spawn_service(service_state, display_pipe);
                }
            }
        }
        return;
    }

    // pid not found in our table — orphan, already reaped, nothing to do.
}

// ---------------------------------------------------------------------------
// State serialization for /proc/bzinit/state
// ---------------------------------------------------------------------------

/// Write a plain-text state snapshot to `fd` (the /proc/bzinit/state fd).
///
/// Format (one line per service):
///   `name status pid=PID retries=N`
pub fn write_state(services: &[ServiceState], state_fd: i32) {
    for service_state in services {
        let line = format!(
            "{} {} pid={} retries={}\n",
            service_state.definition.name,
            service_state.status.as_str(),
            service_state.pid,
            service_state.retry_count,
        );
        raw::raw_write(state_fd, line.as_ptr(), line.len());
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn spawn_service(service_state: &mut ServiceState, display_pipe: Option<&DisplayPipe>) {
    let binary = service_state.definition.binary.clone();
    spawn_with_binary(service_state, binary, display_pipe);
}

fn spawn_with_binary(
    service_state: &mut ServiceState,
    binary: String,
    display_pipe: Option<&DisplayPipe>,
) {
    service_state.status = ServiceStatus::Starting;

    // --- fd setup ---
    // We temporarily redirect bzinit's own stdin/stdout/stderr so that the
    // child process inherits the right fds at spawn time.
    //
    // After spawn returns (the child is already forked), we restore our own
    // fds by dup2-ing back the saved copies.
    //
    // Convention:
    //   fd 0 = stdin   (read end of display pipe → bzdisplayd reads here)
    //   fd 1 = stdout  (write end of display pipe → services write here)
    //   fd 2 = stderr  (same write end, so errors also appear on display)

    let is_display_server = service_state.definition.is_display_server;
    let use_display_output = service_state.definition.display_output;

    if let Some(pipe) = display_pipe {
        if is_display_server {
            // Save bzinit's current stdin, route pipe read end to fd 0.
            let saved_stdin = raw::raw_dup(0);
            raw::raw_dup2(pipe.read_fd, 0);

            do_spawn(service_state, &binary);

            // Restore bzinit's stdin.
            if saved_stdin >= 0 {
                raw::raw_dup2(saved_stdin as i32, 0);
                raw::raw_close(saved_stdin as i32);
            }
            return;
        }

        if use_display_output {
            // Save bzinit's current stdout and stderr, route pipe write end.
            let saved_stdout = raw::raw_dup(1);
            let saved_stderr = raw::raw_dup(2);
            raw::raw_dup2(pipe.write_fd, 1);
            raw::raw_dup2(pipe.write_fd, 2);

            do_spawn(service_state, &binary);

            // Restore bzinit's stdout and stderr.
            if saved_stdout >= 0 {
                raw::raw_dup2(saved_stdout as i32, 1);
                raw::raw_close(saved_stdout as i32);
            }
            if saved_stderr >= 0 {
                raw::raw_dup2(saved_stderr as i32, 2);
                raw::raw_close(saved_stderr as i32);
            }
            return;
        }
    }

    // No fd manipulation needed.
    do_spawn(service_state, &binary);
}

/// Perform the actual spawn syscall and update service state.
fn do_spawn(service_state: &mut ServiceState, binary: &str) {
    let capability_mask = service_state.definition.capabilities;
    let mut binary_buf = [0u8; 512];
    let binary_len = binary.len().min(511);
    binary_buf[..binary_len].copy_from_slice(&binary.as_bytes()[..binary_len]);
    let result = raw::raw_spawn_with_capabilities(
        binary_buf.as_ptr(),
        capability_mask,
    );
    if result < 0 {
        service_state.status = ServiceStatus::Failed;
        let error_output = stderr();
        let _ = error_output.write_all(b"bzinit: failed to spawn '");
        let _ = error_output.write_all(binary.as_bytes());
        let _ = error_output.write_all(b"'\n");
    } else {
        service_state.pid = result as i32;
        service_state.status = ServiceStatus::Running;

        if let Ok(time_spec) = clock_gettime(ClockId::Monotonic) {
            service_state.start_time_seconds = time_spec.seconds;
        }
    }
}

/// Format a u32 as ASCII digits without the standard library.
fn format_u32(value: u32) -> alloc::string::String {
    if value == 0 {
        return alloc::string::String::from("0");
    }
    let mut digits = [0u8; 10];
    let mut position = 10usize;
    let mut remaining = value;
    while remaining > 0 {
        position -= 1;
        digits[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    core::str::from_utf8(&digits[position..])
        .unwrap_or("?")
        .into()
}
