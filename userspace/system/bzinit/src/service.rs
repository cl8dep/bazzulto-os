//! Service definition and runtime state types.

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::capabilities;
use crate::toml_parser::{parse, ParsedToml};

// ---------------------------------------------------------------------------
// Definition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartStrategy {
    /// Never restart on crash.
    Never,
    /// Always restart immediately on crash.
    Always,
    /// Restart with exponential backoff (1s, 2s, 4s, …).
    ExponentialBackoff,
}

impl RestartStrategy {
    fn from_str(string_value: &str) -> RestartStrategy {
        match string_value {
            "always"              => RestartStrategy::Always,
            "exponential-backoff" => RestartStrategy::ExponentialBackoff,
            _                     => RestartStrategy::Never,
        }
    }
}

/// Parsed service definition from a `.service` file.
#[derive(Debug, Clone)]
pub struct ServiceDefinition {
    /// Short name (e.g. `"shell"`).
    pub name: String,
    /// Absolute binary path in ramfs (e.g. `/system/bin/shell`).
    pub binary: String,
    /// Services that must be running before this one starts.
    pub after: Vec<String>,
    /// Maximum number of restart attempts before giving up.
    pub max_retries: u32,
    pub restart_strategy: RestartStrategy,
    /// Alternative binary to try if `max_retries` is exhausted.
    pub fallback: Option<String>,
    /// Capability bitmask to grant the spawned process.
    ///
    /// Parsed from the `capabilities` array in the `.service` file.
    /// Example: `capabilities = ["display"]`
    pub capabilities: u64,
    /// If true, bzinit routes this service's stdout and stderr through the
    /// display pipe so bzdisplayd renders its output on screen.
    ///
    /// Set `display_output = true` in the `[Service]` section.
    pub display_output: bool,
    /// If true, this service IS the display server.  bzinit passes the read
    /// end of the display pipe as its stdin so it can render incoming text.
    pub is_display_server: bool,
    /// Optional user to run this service as (e.g. `"user"`).
    ///
    /// If set, bzinit uses fork + setuid/setgid + exec instead of spawn,
    /// dropping privileges to the specified UID/GID before executing the binary.
    /// The UID/GID are resolved from `/system/config/passwd` at boot time.
    /// If unset, the service inherits bzinit's identity (uid=0, system).
    pub run_as_uid: Option<u32>,
    pub run_as_gid: Option<u32>,
}

impl ServiceDefinition {
    /// Parse a `.service` file from its raw byte content.
    pub fn from_bytes(content: &[u8]) -> Option<ServiceDefinition> {
        let toml = parse(content);
        let name    = toml.get_string("Service", "name")?;
        let binary  = toml.get_string("Service", "binary")?;

        let after: Vec<String> = toml
            .get_array("Service", "after")
            .iter()
            .cloned()
            .collect();

        let max_retries = toml
            .get_string("Service", "max_retries")
            .and_then(|string_value| string_value.parse::<u32>().ok())
            .unwrap_or(3);

        let restart_strategy = RestartStrategy::from_str(
            toml.get_string("Service", "restart").unwrap_or("never"),
        );

        let fallback = toml
            .get_string("Service", "fallback")
            .map(String::from);

        let capabilities = parse_capabilities(
            &toml.get_array("Service", "capabilities"),
        );

        let display_output = toml
            .get_string("Service", "display_output")
            .map(|value| value == "true")
            .unwrap_or(false);

        let is_display_server = toml
            .get_string("Service", "is_display_server")
            .map(|value| value == "true")
            .unwrap_or(false);

        // Optional `user` field: resolve to uid/gid from /system/config/passwd.
        let (run_as_uid, run_as_gid) = match toml.get_string("Service", "user") {
            Some(username) => resolve_user(username),
            None => (None, None),
        };

        Some(ServiceDefinition {
            name: String::from(name),
            binary: String::from(binary),
            after,
            max_retries,
            restart_strategy,
            fallback,
            capabilities,
            display_output,
            is_display_server,
            run_as_uid,
            run_as_gid,
        })
    }
}

// ---------------------------------------------------------------------------
// Runtime state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Pending,
    Starting,
    Running,
    Failed,
    Stopped,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceStatus::Pending  => "pending",
            ServiceStatus::Starting => "starting",
            ServiceStatus::Running  => "running",
            ServiceStatus::Failed   => "failed",
            ServiceStatus::Stopped  => "stopped",
        }
    }
}

/// Runtime state for a single service managed by bzinit.
pub struct ServiceState {
    pub definition: ServiceDefinition,
    pub status: ServiceStatus,
    /// PID of the running process, or 0 if not running.
    pub pid: i32,
    pub retry_count: u32,
    /// Monotonic time (seconds) when the service was last started.
    pub start_time_seconds: u64,
}

impl ServiceState {
    pub fn new(definition: ServiceDefinition) -> ServiceState {
        ServiceState {
            definition,
            status: ServiceStatus::Pending,
            pid: 0,
            retry_count: 0,
            start_time_seconds: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Capability parsing
// ---------------------------------------------------------------------------

/// Translate the `capabilities` string array from a `.service` file into a
/// bitmask using the constants from `bazzulto_system::capabilities`.
///
/// Unknown capability names are silently ignored — forward compatibility.
fn parse_capabilities(names: &[String]) -> u64 {
    let mut mask: u64 = 0;
    for name in names {
        match name.as_str() {
            "display" => mask |= capabilities::CAP_DISPLAY,
            "setcap"  => mask |= capabilities::CAP_SETCAP,
            _         => {} // unknown capability — ignore
        }
    }
    mask
}

/// Resolve a username to (uid, gid) by parsing `/system/config/passwd`.
///
/// Returns `(Some(uid), Some(gid))` on success, `(None, None)` if the user
/// is not found or the file cannot be read.
fn resolve_user(username: &str) -> (Option<u32>, Option<u32>) {
    use bazzulto_system::raw;
    // Read /system/config/passwd
    let mut path_buf = [0u8; 32];
    let path = b"/system/config/passwd\0";
    path_buf[..path.len()].copy_from_slice(path);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return (None, None); }
    let mut buf = [0u8; 1024];
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n <= 0 { return (None, None); }

    // Parse line by line: name:x:uid:gid:...
    let content = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    for line in content.lines() {
        let mut fields = line.splitn(5, ':');
        let name = match fields.next() { Some(n) => n, None => continue };
        if name != username { continue; }
        let _pass = fields.next(); // "x"
        let uid_str = match fields.next() { Some(s) => s, None => continue };
        let gid_str = match fields.next() { Some(s) => s, None => continue };
        let uid: u32 = match uid_str.parse() { Ok(v) => v, Err(_) => continue };
        let gid: u32 = match gid_str.parse() { Ok(v) => v, Err(_) => continue };
        return (Some(uid), Some(gid));
    }
    (None, None)
}
