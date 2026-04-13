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
