// sh/vars.rs — POSIX §2.5 Parameters and Variables
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// §2.5   Parameters and Variables
// §2.5.1 Positional Parameters  ($1, $2, …, ${10}, …)
// §2.5.2 Special Parameters     ($@, $*, $#, $?, $-, $$, $!, $0)
// §2.5.3 Shell Variables        (IFS, PATH, HOME, PS1, PS2, PS4, PWD, …)
//
// This module owns the variable store and provides the accessor/mutator API
// used by builtins (set, unset, export, readonly) and by the expander (§2.6).
//
// The expander (§2.6, future spec) will call `get_param` / `get_special` here.
//
// Variable storage:
//   - Named variables: a flat array of (name, value, flags) entries.
//     No hash map available in no_std without an external crate; linear scan
//     is acceptable for typical shell variable counts (< 100 entries).
//   - Positional parameters: Vec<String> in ShellState, accessed via index.
//   - Special parameters: computed on demand from ShellState fields.
//
// Export semantics:
//   Variables marked exported are passed to child processes via the envp
//   array constructed in executor.rs when fork+exec-ing an external command.
//   (envp construction is a TODO until a getenv/setenv syscall is available.)

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Variable flags
// ---------------------------------------------------------------------------

/// Bitmask flags stored per variable.
pub struct VarFlags(u8);

impl VarFlags {
    pub const NONE:     u8 = 0;
    /// Variable is marked for export to child processes (§2.14 export).
    pub const EXPORT:   u8 = 1 << 0;
    /// Variable is read-only; assignment is an error (§2.14 readonly).
    pub const READONLY: u8 = 1 << 1;

    pub fn new(bits: u8) -> Self { VarFlags(bits) }
    pub fn is_exported(&self)  -> bool { self.0 & Self::EXPORT   != 0 }
    pub fn is_readonly(&self)  -> bool { self.0 & Self::READONLY != 0 }
    pub fn set_export(&mut self)   { self.0 |= Self::EXPORT; }
    pub fn set_readonly(&mut self) { self.0 |= Self::READONLY; }
}

// ---------------------------------------------------------------------------
// Variable entry
// ---------------------------------------------------------------------------

struct VarEntry {
    name:  String,
    value: String,
    flags: VarFlags,
}

// ---------------------------------------------------------------------------
// Variable store
// ---------------------------------------------------------------------------

/// Shell variable store — owns all named variables.
///
/// Positional parameters ($1…$n) and special parameters ($?, $$, etc.) are
/// stored in `ShellState` directly and accessed via `get_positional` /
/// `get_special`, not through this store.
pub struct VarStore {
    vars: Vec<VarEntry>,
}

impl VarStore {
    /// Create an empty variable store.
    pub fn new() -> Self {
        VarStore { vars: Vec::new() }
    }

    /// Initialize from a POSIX envp array (null-terminated array of "KEY=VALUE\0" strings).
    ///
    /// Variables initialized from the environment are immediately marked for
    /// export (§2.5.3: "If a variable is initialized from the environment, it
    /// shall be marked for export immediately").
    ///
    /// # Safety
    /// `envp` must be a valid null-terminated array of null-terminated C strings.
    pub unsafe fn init_from_envp(&mut self, envp: *const *const u8) {
        if envp.is_null() {
            return;
        }
        let mut ptr = envp;
        loop {
            let entry_ptr = *ptr;
            if entry_ptr.is_null() {
                break;
            }
            // Read the null-terminated "KEY=VALUE" string.
            let mut len = 0usize;
            while *entry_ptr.add(len) != 0 {
                len += 1;
            }
            let bytes = core::slice::from_raw_parts(entry_ptr, len);
            // Find '=' in raw bytes — POSIX §8.1 allows arbitrary bytes in values.
            if let Some(eq) = bytes.iter().position(|&b| b == b'=') {
                let key_bytes   = &bytes[..eq];
                let value_bytes = &bytes[eq + 1..];
                // Keys must be valid UTF-8 names.
                if let Ok(name) = core::str::from_utf8(key_bytes) {
                    if is_valid_name(name) {
                        // Values are decoded lossily; non-UTF-8 bytes become U+FFFD.
                        let value = alloc::string::String::from_utf8_lossy(value_bytes);
                        self.set_exported(name, &value);
                    }
                }
            }
            ptr = ptr.add(1);
        }
    }

    /// Get the value of a named variable. Returns None if unset.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.iter()
            .rfind(|e| e.name == name)
            .map(|e| e.value.as_str())
    }

    /// Set a variable. Creates it if it doesn't exist.
    ///
    /// Returns `Err` if the variable is read-only.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), &'static str> {
        if let Some(entry) = self.vars.iter_mut().rfind(|e| e.name == name) {
            if entry.flags.is_readonly() {
                return Err("read-only variable");
            }
            entry.value = value.to_string();
            return Ok(());
        }
        self.vars.push(VarEntry {
            name:  name.to_string(),
            value: value.to_string(),
            flags: VarFlags::new(VarFlags::NONE),
        });
        Ok(())
    }

    /// Set a variable and mark it for export.
    pub fn set_exported(&mut self, name: &str, value: &str) {
        if let Some(entry) = self.vars.iter_mut().rfind(|e| e.name == name) {
            entry.value = value.to_string();
            entry.flags.set_export();
            return;
        }
        let mut flags = VarFlags::new(VarFlags::NONE);
        flags.set_export();
        self.vars.push(VarEntry {
            name:  name.to_string(),
            value: value.to_string(),
            flags,
        });
    }

    /// Mark an existing variable for export (or create an empty exported var).
    pub fn export(&mut self, name: &str) {
        if let Some(entry) = self.vars.iter_mut().rfind(|e| e.name == name) {
            entry.flags.set_export();
            return;
        }
        // Export a variable that doesn't exist yet — creates it as empty.
        let mut flags = VarFlags::new(VarFlags::NONE);
        flags.set_export();
        self.vars.push(VarEntry {
            name:  name.to_string(),
            value: String::new(),
            flags,
        });
    }

    /// Mark a variable as read-only. Creates it (empty, not exported) if absent.
    pub fn set_readonly(&mut self, name: &str) {
        if let Some(entry) = self.vars.iter_mut().rfind(|e| e.name == name) {
            entry.flags.set_readonly();
            return;
        }
        let mut flags = VarFlags::new(VarFlags::NONE);
        flags.set_readonly();
        self.vars.push(VarEntry {
            name:  name.to_string(),
            value: String::new(),
            flags,
        });
    }

    /// Unset a variable. No-op if not set.
    /// Returns `Err` if the variable is read-only (§2.14 unset).
    pub fn unset(&mut self, name: &str) -> Result<(), &'static str> {
        if let Some(pos) = self.vars.iter().rposition(|e| e.name == name) {
            if self.vars[pos].flags.is_readonly() {
                return Err("read-only variable");
            }
            self.vars.remove(pos);
        }
        Ok(())
    }

    /// Returns true if the variable is set (even if its value is empty).
    pub fn is_set(&self, name: &str) -> bool {
        self.vars.iter().any(|e| e.name == name)
    }

    /// Iterate over all exported variables, calling `f(name, value)` for each.
    pub fn for_each_exported<F: FnMut(&str, &str)>(&self, mut f: F) {
        for entry in &self.vars {
            if entry.flags.is_exported() {
                f(entry.name.as_str(), entry.value.as_str());
            }
        }
    }

    /// Build a flat `NAME=value\0` byte array for `execve`-style environments.
    ///
    /// Returns a tuple of:
    ///   - `flat`: contiguous byte buffer of null-terminated `NAME=value` strings
    ///   - `offsets`: byte offset of each string start within `flat`
    ///
    /// The caller builds a pointer array of `flat.as_ptr() + offsets[i]` for
    /// each entry, then passes a null-terminated pointer array to `raw_exec`.
    pub fn build_envp(&self) -> (Vec<u8>, Vec<usize>) {
        let mut flat: Vec<u8> = Vec::new();
        let mut offsets: Vec<usize> = Vec::new();
        for entry in &self.vars {
            if !entry.flags.is_exported() { continue; }
            offsets.push(flat.len());
            flat.extend_from_slice(entry.name.as_bytes());
            flat.push(b'=');
            flat.extend_from_slice(entry.value.as_bytes());
            flat.push(b'\0');
        }
        (flat, offsets)
    }
}

// ---------------------------------------------------------------------------
// Special parameter expansion (§2.5.2)
// ---------------------------------------------------------------------------

/// Expand a special parameter character to its string value.
///
/// Called by the expander (§2.6, future spec) when it encounters `$X` where
/// X is one of: @ * # ? - $ ! 0
///
/// `state` provides positional params, exit status, shell name, and PID.
pub fn expand_special(ch: char, state: &crate::ShellState) -> String {
    match ch {
        // $@ — positional parameters as separate fields (§2.5.2).
        // In non-double-quote context: space-separated (same as $* here,
        // field splitting differences handled by the expander).
        '@' | '*' => {
            state.positional_params.join(" ")
        }

        // $# — number of positional parameters (§2.5.2).
        '#' => {
            format_u32(state.positional_params.len() as u32)
        }

        // $? — exit status of most recently executed pipeline (§2.5.2).
        '?' => {
            format_i32(state.last_exit_status)
        }

        // $- — current option flags (§2.5.2).
        // We track no option flags currently; return empty string.
        '-' => String::new(),

        // $$ — PID of the shell (§2.5.2).
        '$' => {
            format_u32(state.shell_pid)
        }

        // $! — PID of most recent background command (§2.5.2).
        '!' => {
            match state.last_background_pid {
                Some(pid) => format_u32(pid as u32),
                None      => String::new(),
            }
        }

        // $0 — name of the shell or script (§2.5.2).
        '0' => state.shell_name.clone(),

        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Variable name validation (§2.5 + XBD Name definition)
// ---------------------------------------------------------------------------

/// Returns true if `name` is a valid shell variable name.
///
/// POSIX XBD "Name": `[_a-zA-Z][_a-zA-Z0-9]*`
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Parse a `NAME=value` assignment word.
///
/// Returns `Some((name, value))` if the word is a valid assignment.
/// Returns `None` if the word is not an assignment or `name` is invalid.
pub fn parse_assignment(word: &str) -> Option<(&str, &str)> {
    let eq = word.find('=')?;
    let name = &word[..eq];
    if !is_valid_name(name) {
        return None;
    }
    Some((name, &word[eq + 1..]))
}

// ---------------------------------------------------------------------------
// Helpers — integer formatting without std
// ---------------------------------------------------------------------------

/// Format a u32 as a decimal string.
pub fn format_u32(n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = [0u8; 10];
    let mut pos = 10usize;
    let mut val = n;
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    core::str::from_utf8(&buf[pos..]).unwrap_or("0").to_string()
}

/// Format an i32 as a decimal string.
pub fn format_i32(n: i32) -> String {
    if n >= 0 {
        format_u32(n as u32)
    } else {
        let mut s = String::from("-");
        s.push_str(&format_u32((-(n as i64)) as u32));
        s
    }
}
