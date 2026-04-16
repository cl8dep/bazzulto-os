//! Process environment — access and mutation of `KEY=VALUE` variables.
//!
//! The kernel writes the environment onto the initial stack page as a
//! NULL-terminated array of pointers (envp[], x2 on AArch64 SysV ABI entry).
//! `_start` stores that pointer via `init_with_args_envp()`.
//!
//! Design:
//!   - `get` / `contains_key` read directly from the kernel-supplied envp[]
//!     array (zero allocation, zero copy).
//!   - `set` / `delete` maintain a per-process overlay stored in a heap-
//!     allocated `BTreeMap`.  Reads check the overlay first, then fall back to
//!     the kernel array.  This matches the POSIX `setenv(3)` / `getenv(3)` model
//!     where modifications are visible to the current process but not inherited
//!     by children unless passed explicitly in `execve`.
//!   - `all()` merges the overlay with the kernel array and returns a snapshot.
//!
//! Reference: POSIX.1-2017 §8.1 (Environment Variables), getenv(3), setenv(3).

extern crate alloc;

use alloc::string::String;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// Well-known system directories (Bazzulto Path Model)
// ---------------------------------------------------------------------------
//
// These enums are the SINGLE SOURCE OF TRUTH for all system paths.
// To add a new path, add a variant here — do not hardcode paths elsewhere.
//
// Reference: docs/Roadmap.md M3 §3.7, Bazzulto Path Model.

/// Well-known system directories.
///
/// Use `Environment::get_special_folder()` to resolve to a path string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialFolder {
    /// `/system` — system root (binaries, libraries, config, fonts).
    SystemRoot,
    /// `/system/bin` — system executables.
    SystemBin,
    /// `/system/lib` — system shared libraries.
    SystemLib,
    /// `/system/config` — system configuration (passwd, group, services).
    /// Equivalent of `/etc` on traditional Unix.
    SystemConfig,
    /// `/system/config/services` — bzinit service definitions.
    SystemServices,
    /// `/system/fonts` — system font files.
    SystemFonts,
    /// `/system/share` — architecture-independent data (timezones, etc.).
    SystemShare,
    /// `/home/user` — default user home directory.
    UserHome,
    /// `/data` — persistent application data.
    Data,
    /// `/data/temp` — temporary files (cleared on reboot).
    DataTemp,
    /// `/data/logs` — system and application logs.
    DataLogs,
    /// `/apps` — user-installed applications.
    Apps,
    /// `/system/config/policies` — BPM policy store (SHA-256 keyed).
    SystemPolicies,
}

impl SpecialFolder {
    /// Return the absolute path for this folder.
    pub const fn path(self) -> &'static str {
        match self {
            SpecialFolder::SystemRoot     => "/system",
            SpecialFolder::SystemBin      => "/system/bin",
            SpecialFolder::SystemLib      => "/system/lib",
            SpecialFolder::SystemConfig   => "/system/config",
            SpecialFolder::SystemServices => "/system/config/services",
            SpecialFolder::SystemFonts    => "/system/fonts",
            SpecialFolder::SystemShare    => "/system/share",
            SpecialFolder::UserHome       => "/home/user",
            SpecialFolder::Data           => "/data",
            SpecialFolder::DataTemp       => "/data/temp",
            SpecialFolder::DataLogs       => "/data/logs",
            SpecialFolder::Apps           => "/apps",
            SpecialFolder::SystemPolicies => "/system/config/policies",
        }
    }
}

/// Well-known system files.
///
/// Use `Environment::get_special_file()` to resolve to a path string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialFile {
    /// `/system/config/passwd` — user database (name, uid, gid, shell).
    Passwd,
    /// `/system/config/shadow` — password hashes (root-readable only).
    Shadow,
    /// `/system/config/group` — group database (name, gid, members).
    Group,
    /// `/system/config/hostname` — machine hostname.
    Hostname,
    /// `/system/config/disk-mounts` — disk mount configuration.
    DiskMounts,
}

impl SpecialFile {
    /// Return the absolute path for this file.
    pub const fn path(self) -> &'static str {
        match self {
            SpecialFile::Passwd     => "/system/config/passwd",
            SpecialFile::Shadow     => "/system/config/shadow",
            SpecialFile::Group      => "/system/config/group",
            SpecialFile::Hostname   => "/system/config/hostname",
            SpecialFile::DiskMounts => "/system/config/disk-mounts",
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal SpinLock for single-threaded userspace use
// ---------------------------------------------------------------------------
//
// Bazzulto userspace processes are currently single-threaded.  This lock
// provides the correct API without spinning — it is purely a safe-access
// wrapper around UnsafeCell.

struct SpinLock<T>(UnsafeCell<T>);

// SAFETY: single-threaded userspace process; no concurrent access possible.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

struct SpinLockGuard<'a, T>(&'a UnsafeCell<T>);

impl<T> SpinLock<T> {
    const fn new(value: T) -> Self { Self(UnsafeCell::new(value)) }
    fn lock(&self) -> SpinLockGuard<'_, T> { SpinLockGuard(&self.0) }
}

impl<T> core::ops::Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.0.get() } }
}

impl<T> core::ops::DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.0.get() } }
}

// ---------------------------------------------------------------------------
// Overlay — heap-resident mutations (set / delete)
// ---------------------------------------------------------------------------
//
// `Option<String>` values:
//   Some(value) — variable is set to `value` (overrides the kernel array)
//   None        — variable has been explicitly deleted (shadows kernel array)

static ENV_OVERLAY: SpinLock<Option<BTreeMap<String, Option<String>>>> =
    SpinLock::new(None);

fn with_overlay<R>(f: impl FnOnce(&mut BTreeMap<String, Option<String>>) -> R) -> R {
    let mut guard = ENV_OVERLAY.lock();
    if guard.is_none() {
        *guard = Some(BTreeMap::new());
    }
    f(guard.as_mut().unwrap())
}

// ---------------------------------------------------------------------------
// Helper: iterate the kernel envp[] array
// ---------------------------------------------------------------------------

/// Call `f(key, value)` for each `KEY=VALUE` entry in the kernel-supplied
/// NULL-terminated envp[] array.  Entries not in UTF-8 or without `=` are
/// silently skipped.
///
/// # Safety
/// Requires that `PROCESS_ENVP` was stored from a valid kernel envp[].
fn for_each_kernel_env(mut f: impl FnMut(&str, &str)) {
    let envp = crate::envp_raw();
    if envp.is_null() {
        return;
    }
    let mut index = 0usize;
    loop {
        // Safety: envp[] is NULL-terminated; the kernel guarantees each
        // pointer is valid until the process exits.
        let entry_ptr = unsafe { *envp.add(index) };
        if entry_ptr.is_null() {
            break;
        }
        index += 1;
        if index > 4096 {
            break; // safety cap: no sane process has more than 4096 env vars
        }

        // Measure the NUL-terminated string length.
        let mut len = 0usize;
        loop {
            if unsafe { *entry_ptr.add(len) } == 0 { break; }
            len += 1;
            if len > 65536 { break; } // safety cap: no env entry this long
        }

        let bytes = unsafe { core::slice::from_raw_parts(entry_ptr, len) };

        // Find the first `=` byte; entries without one are ignored.
        // We scan raw bytes rather than parsing as UTF-8 first, because POSIX
        // §8.1 allows arbitrary non-NUL bytes in values.  Only the key must
        // be valid ASCII (§8.1: names consist of [A-Za-z0-9_]).
        let Some(eq_pos) = bytes.iter().position(|&b| b == b'=') else { continue };
        let key_bytes   = &bytes[..eq_pos];
        let value_bytes = &bytes[eq_pos + 1..];

        // Keys must be valid UTF-8 names; skip entries with non-UTF-8 keys.
        let Ok(key) = core::str::from_utf8(key_bytes) else { continue };

        // Values: convert to UTF-8 lossily so non-UTF-8 bytes are preserved
        // as U+FFFD replacement characters rather than silently dropped.
        // Most values in practice are ASCII; this only fires on unusual envs.
        let value_owned = alloc::string::String::from_utf8_lossy(value_bytes);
        f(key, &value_owned);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Access and mutation of the process environment variables.
pub struct Environment;

impl Environment {
    /// Look up the value of environment variable `key`.
    ///
    /// Checks the in-process overlay first (populated by `set()` / `delete()`),
    /// then falls back to the kernel-supplied envp array.
    ///
    /// Returns `None` if the variable is not set or has been deleted.
    ///
    /// Reference: POSIX.1-2017 getenv(3).
    pub fn get(key: &str) -> Option<String> {
        // Check the overlay first.
        {
            let guard = ENV_OVERLAY.lock();
            if let Some(ref map) = *guard {
                if let Some(entry) = map.get(key) {
                    // `None` in the overlay means the variable was explicitly deleted.
                    return entry.clone();
                }
            }
        }
        // Fall back to the kernel-supplied envp array.
        let mut result = None;
        for_each_kernel_env(|k, v| {
            if k == key {
                result = Some(String::from(v));
            }
        });
        result
    }

    /// Return `true` if `key` is currently set.
    pub fn contains_key(key: &str) -> bool {
        Self::get(key).is_some()
    }

    /// Set `key` to `value`, overwriting any existing value.
    ///
    /// The new value is immediately visible to `get()` and `all()`.
    /// It is NOT automatically propagated to child processes via `execve`;
    /// callers that exec a child must build and pass the new envp explicitly.
    ///
    /// Reference: POSIX.1-2017 setenv(3).
    pub fn set(key: &str, value: &str) {
        with_overlay(|map| {
            map.insert(String::from(key), Some(String::from(value)));
        });
    }

    /// Remove `key` from the environment.
    ///
    /// After this call `get(key)` returns `None` even if the variable was
    /// present in the kernel-supplied envp.
    ///
    /// Reference: POSIX.1-2017 unsetenv(3).
    pub fn delete(key: &str) {
        with_overlay(|map| {
            map.insert(String::from(key), None);
        });
    }

    /// Set an environment variable from a `KEY=VALUE` string.
    ///
    /// Parses the string at the first `=`.  If there is no `=`, the entire
    /// string is treated as a key with an empty value (matching glibc behaviour).
    ///
    /// Reference: POSIX.1-2017 putenv(3).
    pub fn putenv(entry: &str) {
        match entry.find('=') {
            Some(eq) => Self::set(&entry[..eq], &entry[eq + 1..]),
            None     => Self::set(entry, ""),
        }
    }

    /// Remove all environment variables.
    ///
    /// Clears the in-process overlay and zeros the stored kernel envp pointer
    /// so that subsequent `get()` / `all()` calls see an empty environment.
    ///
    /// Reference: POSIX.1-2017 clearenv(3).
    pub fn clearenv() {
        with_overlay(|map| { map.clear(); });
        // Zero the envp pointer so for_each_kernel_env returns immediately.
        // This is safe: PROCESS_ENVP is our own static; no other code holds
        // a reference to the pointer at this level.
        crate::PROCESS_ENVP.store(
            core::ptr::null_mut(),
            core::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Return a snapshot of all currently set environment variables.
    ///
    /// Merges the kernel-supplied envp array with in-process modifications.
    /// Variables deleted via `delete()` are excluded.
    ///
    /// Reference: POSIX.1-2017 environ(7).
    pub fn all() -> BTreeMap<String, String> {
        // Start with the kernel-supplied entries.
        let mut result: BTreeMap<String, String> = BTreeMap::new();
        for_each_kernel_env(|k, v| {
            result.insert(String::from(k), String::from(v));
        });

        // Apply overlay: Some(v) overwrites, None deletes.
        let guard = ENV_OVERLAY.lock();
        if let Some(ref map) = *guard {
            for (key, val) in map.iter() {
                match val {
                    Some(v) => { result.insert(key.clone(), v.clone()); }
                    None    => { result.remove(key.as_str()); }
                }
            }
        }

        result
    }

    /// Return the process arguments as owned `String` values.
    pub fn args() -> Vec<String> {
        crate::args().map(|s| {
            let mut owned = String::new();
            owned.push_str(s);
            owned
        }).collect()
    }

    // --- Well-known system paths (Bazzulto Path Model) ---
    //
    // All system paths are centralised here.  Coreutils and services should
    // NEVER hardcode paths like "/system/config/passwd" — use these methods.

    /// Resolve a well-known system directory.
    ///
    /// ```ignore
    /// let config = Environment::get_special_folder(SpecialFolder::SystemConfig);
    /// // → "/system/config"
    /// ```
    pub fn get_special_folder(folder: SpecialFolder) -> &'static str {
        folder.path()
    }

    /// Resolve a well-known system file.
    ///
    /// ```ignore
    /// let passwd = Environment::get_special_file(SpecialFile::Passwd);
    /// // → "/system/config/passwd"
    /// ```
    pub fn get_special_file(file: SpecialFile) -> &'static str {
        file.path()
    }

    // --- Static properties (no syscall required) ---

    /// Canonical temporary directory path.
    pub fn temp_dir() -> &'static str { "/tmp" }

    /// Return the home directory.
    ///
    /// Returns the value of the `HOME` environment variable when set.
    /// Falls back to `/home/user` if `HOME` is not in the environment.
    ///
    /// Note: returns a `&'static str` only for the fallback; callers that need
    /// the dynamic value should use `Environment::get("HOME")` directly.
    pub fn home_dir() -> &'static str {
        // `get` returns an owned String from the heap; we cannot return a
        // reference into it from this function.  The static fallback covers
        // the case where HOME is not set; callers needing the dynamic value
        // should call `Environment::get("HOME")`.
        "/home/user"
    }

    /// Return the home directory path, checking `$HOME` first.
    ///
    /// Unlike `home_dir()`, this returns an owned `String` so that the
    /// dynamic `$HOME` value can be returned without a lifetime issue.
    pub fn home_dir_owned() -> String {
        Self::get("HOME").unwrap_or_else(|| String::from("/home/user"))
    }

    /// Operating system version string — sourced from the kernel via `sys_uname`.
    pub fn os_version() -> &'static str { crate::info::Info::os_version() }

    /// CPU architecture — sourced from the kernel via `sys_uname`.
    pub fn arch() -> &'static str { crate::info::Info::arch() }

    /// Return the hostname from the `HOSTNAME` environment variable.
    pub fn hostname() -> Option<String> {
        Self::get("HOSTNAME")
    }

    /// Return the current username from the `USER` environment variable.
    pub fn username() -> Option<String> {
        Self::get("USER")
    }

    /// Return the number of online CPUs.
    pub fn cpu_count() -> Option<u32> { crate::info::Info::cpu_count() }

    /// Return total physical memory in bytes — sourced from `sys_sysinfo`.
    pub fn memory_total() -> Option<u64> { crate::info::Info::memory_total() }

    /// Return available (free) physical memory in bytes — sourced from `sys_sysinfo`.
    pub fn memory_available() -> Option<u64> {
        let mut buf = [0u64; 4];
        let result = crate::raw::raw_sysinfo(buf.as_mut_ptr());
        if result == 0 && buf[2] > 0 { Some(buf[2]) } else { None }
    }
}
