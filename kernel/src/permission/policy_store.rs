// permission/policy_store.rs — In-memory policy store for the BPM.
//
// Stores per-binary permission grants keyed by SHA-256 hash.
// Loaded from /system/config/policies/ at boot, persisted on grant.
//
// Two key variants:
//   {sha256_hex}         — system-wide grant (Authenticated level)
//   {sha256_hex}:{uid}   — per-user grant (Consent level)
//
// Policy file format (plain text):
//   binary_path = /home/user/myapp
//   granted_permissions = //user:/home/**, //ram:/tmp/**
//   grant_scope = permanent
//   granted_by_uid = 0
//   granted_at = 1700000000

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

use super::PathPattern;

/// A single policy entry for a binary.
#[derive(Clone, Debug)]
pub struct PolicyEntry {
    /// SHA-256 hex string of the binary's PT_LOAD segments.
    pub hash_hex: String,
    /// Original binary path (informational, not used for matching).
    pub binary_path: String,
    /// Granted access permission patterns.
    pub granted_permissions: Vec<PathPattern>,
    /// Grant scope: "permanent", "session", or "once".
    pub grant_scope: String,
    /// UID that granted this policy (0 = system-wide).
    pub granted_by_uid: u32,
    /// Unix timestamp of grant time.
    pub granted_at: u64,
}

/// The in-memory policy store.
///
/// Keyed by SHA-256 hex string.  System-wide entries use the bare hash;
/// per-user entries append ":{uid}".
pub struct PolicyStore {
    entries: BTreeMap<String, PolicyEntry>,
}

impl PolicyStore {
    pub const fn new() -> Self {
        PolicyStore { entries: BTreeMap::new() }
    }

    /// Look up a policy by binary hash (system-wide).
    pub fn lookup(&self, hash_hex: &str) -> Option<&PolicyEntry> {
        self.entries.get(hash_hex)
    }

    /// Look up a per-user policy by binary hash and UID.
    pub fn lookup_for_user(&self, hash_hex: &str, uid: u32) -> Option<&PolicyEntry> {
        let mut key_buf = String::with_capacity(hash_hex.len() + 12);
        key_buf.push_str(hash_hex);
        key_buf.push(':');
        // Format uid as decimal.
        let mut uid_buf = [0u8; 10];
        let uid_str = format_u32(uid, &mut uid_buf);
        key_buf.push_str(uid_str);
        self.entries.get(&key_buf)
    }

    /// Look up policy: try per-user first, then system-wide.
    pub fn lookup_best(&self, hash_hex: &str, uid: u32) -> Option<&PolicyEntry> {
        self.lookup_for_user(hash_hex, uid)
            .or_else(|| self.lookup(hash_hex))
    }

    /// Insert or replace a policy entry.
    pub fn insert(&mut self, key: String, entry: PolicyEntry) {
        self.entries.insert(key, entry);
    }

    /// Remove a policy entry by key.
    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Iterate all entries (for bzpermctl list).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PolicyEntry)> {
        self.entries.iter()
    }

    /// Parse a policy file's content and insert the entry.
    ///
    /// Returns `true` if parsing succeeded.
    pub fn load_from_text(&mut self, hash_key: &str, content: &str) -> bool {
        let mut binary_path = String::new();
        let mut permissions: Vec<PathPattern> = Vec::new();
        let mut grant_scope = String::from("permanent");
        let mut granted_by_uid: u32 = 0;
        let mut granted_at: u64 = 0;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "binary_path" => binary_path = String::from(value),
                    "granted_permissions" => {
                        for pattern in value.split(',') {
                            let p = pattern.trim();
                            if !p.is_empty() {
                                permissions.push(PathPattern::new(String::from(p)));
                            }
                        }
                    }
                    "grant_scope" => grant_scope = String::from(value),
                    "granted_by_uid" => granted_by_uid = value.parse().unwrap_or(0),
                    "granted_at" => granted_at = value.parse().unwrap_or(0),
                    _ => {} // unknown field — ignore
                }
            }
        }

        if permissions.is_empty() { return false; }

        self.insert(String::from(hash_key), PolicyEntry {
            hash_hex: String::from(hash_key),
            binary_path,
            granted_permissions: permissions,
            grant_scope,
            granted_by_uid,
            granted_at,
        });
        true
    }
}

fn format_u32(mut val: u32, buf: &mut [u8; 10]) -> &str {
    if val == 0 { return "0"; }
    let mut i = 9;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        if i == 0 { break; }
        i -= 1;
    }
    core::str::from_utf8(&buf[i + 1..]).unwrap_or("0")
}
