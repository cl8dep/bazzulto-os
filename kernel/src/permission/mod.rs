// permission/mod.rs — Bazzulto Binary Permission Model (kernel side).
//
// This module implements the parts of the Binary Permission Model that live
// entirely in the kernel, without requiring permissiond or Ed25519 signing.
//
// # Overview
//
// The model has two orthogonal permission dimensions:
//
//   Access permissions — canonical path patterns such as `//user:/**` or
//     `//net:**`.  Checked in `sys_open()` before inode lookup (anti-
//     enumeration: a denied path returns EACCES *before* checking whether
//     the inode exists).
//
//   Action permissions — capability tokens such as `MountFilesystem` or
//     `LoadKernelDriver`.  Checked in the syscalls that perform the
//     corresponding operations.
//
// # Trust tiers (v1.0 kernel-side implementation)
//
//   Tier 1 (system binaries): any binary whose resolved canonical path starts
//     with `//system:/bin/` or `//system:/sbin/` receives the wildcard
//     permission set `[//:**]` (full access) at exec time.  Ed25519
//     signature verification is deferred to post-v1.0.
//
//   Tier 4 (no declaration): if a binary does not carry a
//     `.bazzulto_permissions` ELF section, it inherits the parent's
//     `granted_permissions` and `granted_actions` unchanged, and a warning
//     is written to the process's stderr.  bzinit is launched with the full
//     wildcard set, so all its children inherit full trust transitively.
//
//   Tier 2/3 (policy store + prompt): requires permissiond.  Deferred to
//     post-v1.0.  The infrastructure (`granted_permissions`, `granted_actions`
//     fields) is in place for permissiond to populate.
//
// # Reference
//   docs/features/Binary Permission Model.md

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// ApprovalLevel — how much user interaction granting a permission requires
// ---------------------------------------------------------------------------

/// How much user interaction is required to grant a permission.
///
/// The level is determined by the *most sensitive* namespace that a permission
/// pattern touches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApprovalLevel {
    /// No user interaction required.  Tier-4 inherited permissions are silent.
    Silent,
    /// A UI consent click is required (no password).
    ///
    /// Example: `//user:/**`, `//dev:**`, `//net:**`.
    Consent,
    /// A UI click plus the user's password.  Admin-group only.
    ///
    /// Example: `//sys:mount/**`, `//sys:driver/**`.
    Authenticated,
    /// Hard-reject: this permission can never be granted to a userspace process
    /// via the normal permission request flow.
    ///
    /// The kernel returns EPERM regardless of what `granted_permissions` contains.
    /// Example: `//sys:policy:write/**`, `//sys:perm:grant/**`.
    Impossible,
}

// ---------------------------------------------------------------------------
// Access-permission namespace sensitivity table
// ---------------------------------------------------------------------------

/// Static table mapping canonical path prefixes to their minimum
/// `ApprovalLevel`.  The first matching prefix wins (most-specific first).
///
/// Entries ending in `:` cover the entire namespace scheme regardless of path.
static NAMESPACE_SENSITIVITY: &[(&str, ApprovalLevel)] = &[
    // Hard-rejects: modifying policy or granting permissions — never allowed.
    ("//sys:policy:",      ApprovalLevel::Impossible),
    ("//sys:perm:",        ApprovalLevel::Impossible),
    // System binary trees are write-protected (reads from userspace are allowed).
    ("//system:/bin/",     ApprovalLevel::Impossible),  // write
    ("//system:/sbin/",    ApprovalLevel::Impossible),  // write
    // System-level mount/driver/kernel actions — Authenticated via action perms.
    ("//sys:",             ApprovalLevel::Authenticated),
    // User data, network, temporary storage, and peripheral devices — Consent.
    ("//user:",            ApprovalLevel::Consent),
    ("//net:",             ApprovalLevel::Consent),
    ("//ram:",             ApprovalLevel::Consent),
    ("//dev:",             ApprovalLevel::Consent),
];

// ---------------------------------------------------------------------------
// PathPattern — a compiled canonical-path glob pattern
// ---------------------------------------------------------------------------

/// A compiled canonical path pattern.
///
/// Patterns follow the Bazzulto canonical path grammar:
///   `//scheme:authority/path/**`   — match everything under a directory.
///   `//scheme:authority/path/*`    — match direct children only.
///   `//scheme:**`                  — match everything in a namespace.
///   `//:**`                        — wildcard: match any canonical path.
///
/// Matching is prefix-based for `**` suffixes and exact for `*` (one
/// non-`/` segment).
#[derive(Clone, Debug)]
pub struct PathPattern {
    raw: String,
}

impl PathPattern {
    /// Create a pattern from a raw string.  No validation is performed; an
    /// ill-formed pattern simply never matches.
    pub fn new(raw: String) -> Self {
        PathPattern { raw }
    }

    /// Create the wildcard pattern `//:**` that matches every canonical path.
    pub fn wildcard() -> Self {
        PathPattern { raw: String::from("//:**") }
    }

    /// Return `true` if this pattern matches `canonical_path`.
    ///
    /// Matching rules:
    ///   - `//:**` matches everything.
    ///   - Pattern ending in `/**` → prefix match on the part before `/**`.
    ///   - Pattern ending in `:**` → prefix match on the scheme+colon.
    ///   - Pattern ending in `/*`  → parent path must match and the last
    ///     component must contain no `/`.
    ///   - Otherwise: exact match.
    pub fn matches(&self, canonical_path: &str) -> bool {
        let pattern = self.raw.as_str();

        // Universal wildcard.
        if pattern == "//:**" || pattern == "//**" {
            return true;
        }

        if let Some(prefix) = pattern.strip_suffix("/**") {
            // Match everything under `prefix/`.
            return canonical_path == prefix
                || canonical_path.starts_with(prefix)
                    && canonical_path[prefix.len()..].starts_with('/');
        }

        if let Some(prefix) = pattern.strip_suffix(":**") {
            // Match everything in the scheme namespace `prefix:`.
            let full_prefix = alloc::format!("{}:", prefix);
            return canonical_path.starts_with(&full_prefix);
        }

        if let Some(prefix) = pattern.strip_suffix("/*") {
            // Match direct children only (no `/` after the separator).
            if !canonical_path.starts_with(prefix) {
                return false;
            }
            let rest = &canonical_path[prefix.len()..];
            if !rest.starts_with('/') {
                return false;
            }
            // The child name (after the `/`) must contain no further `/`.
            let child_name = &rest[1..];
            return !child_name.is_empty() && !child_name.contains('/');
        }

        // Exact match.
        pattern == canonical_path
    }

    /// Return the raw pattern string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

// ---------------------------------------------------------------------------
// ActionPermission — OS-level capability tokens
// ---------------------------------------------------------------------------

/// OS-level capability tokens for actions that require explicit approval.
///
/// These are distinct from access permissions (path patterns) — they gate
/// specific privileged kernel operations rather than filesystem paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionPermission {
    /// `sys_mount()` / `sys_umount()` — Authenticated.
    MountFilesystem,
    /// Future driver-load syscall — Authenticated.
    LoadKernelDriver,
    /// Future network-configuration syscall — Authenticated.
    ModifyNetworkConfig,
    /// Package-manager install operations — Authenticated.
    InstallPackage,
    /// Future `sys_sysctl()` — Authenticated.
    ModifyKernelParams,
    /// Hard-reject: only permissiond may hold this, and the kernel never grants it.
    GrantPermissions,
}

impl ActionPermission {
    /// Return the approval level required to hold this action permission.
    pub fn required_approval(self) -> ApprovalLevel {
        match self {
            Self::MountFilesystem     => ApprovalLevel::Authenticated,
            Self::LoadKernelDriver    => ApprovalLevel::Authenticated,
            Self::ModifyNetworkConfig => ApprovalLevel::Authenticated,
            Self::InstallPackage      => ApprovalLevel::Authenticated,
            Self::ModifyKernelParams  => ApprovalLevel::Authenticated,
            Self::GrantPermissions    => ApprovalLevel::Impossible,
        }
    }
}

// ---------------------------------------------------------------------------
// Permission check functions
// ---------------------------------------------------------------------------

/// Return `true` if the set of `granted` patterns allows access to
/// `canonical_path`.
///
/// If `granted` is empty, access is allowed (Tier-4 transitional mode: no
/// declaration means "inherit everything").
///
/// The check is pure pattern matching — no inode lookup, no disk I/O.
pub fn permission_allows(granted: &[PathPattern], canonical_path: &str) -> bool {
    if granted.is_empty() {
        // Tier-4 transitional mode: no declared permissions → bypass.
        return true;
    }
    granted.iter().any(|pattern| pattern.matches(canonical_path))
}

/// Return `true` if `canonical_path` is in an `Impossible` namespace.
///
/// This check is performed regardless of `granted_permissions`.  Even if a
/// process somehow has the wildcard pattern, Impossible namespaces are always
/// denied.
pub fn is_impossible_namespace(canonical_path: &str) -> bool {
    NAMESPACE_SENSITIVITY.iter().any(|(prefix, level)| {
        *level == ApprovalLevel::Impossible && canonical_path.starts_with(prefix)
    })
}

/// Check whether the process has the given `ActionPermission`.
///
/// Returns:
///   - `Ok(())` — permission granted.
///   - `Err(PermissionError::Impossible)` — `GrantPermissions` requested;
///     always denied regardless of `granted_actions`.
///   - `Err(PermissionError::Denied)` — action not in `granted_actions`.
pub fn check_action_permission(
    granted_actions: &[ActionPermission],
    action: ActionPermission,
) -> Result<(), PermissionError> {
    if action == ActionPermission::GrantPermissions {
        return Err(PermissionError::Impossible);
    }
    if granted_actions.contains(&action) {
        Ok(())
    } else {
        Err(PermissionError::Denied)
    }
}

/// Error returned when a permission check fails.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermissionError {
    /// The process does not have the requested permission.
    Denied,
    /// The requested operation is unconditionally forbidden by the kernel.
    Impossible,
}

impl PermissionError {
    /// Convert to POSIX errno.
    pub fn to_errno(self) -> i64 {
        match self {
            PermissionError::Denied    => -13, // EACCES
            PermissionError::Impossible => -1, // EPERM
        }
    }
}

// ---------------------------------------------------------------------------
// ELF section detection helper
// ---------------------------------------------------------------------------

/// Check whether the ELF binary in `elf_data` contains a section named
/// `.bazzulto_permissions`.
///
/// Returns `true` if the section exists (Tier 2/3), `false` if it is absent
/// (Tier 4 — no declaration).
///
/// This function reads only the section header table and the section-name
/// string table.  It does not interpret the section contents (that is
/// permissiond's job in post-v1.0).  It only answers the yes/no question
/// that determines whether Tier-4 inheritance applies.
///
/// Returns `false` on any parse error (treat as Tier 4).
///
/// Reference: ELF-64 Object File Format v1.5, §3 (Section Headers).
pub fn elf_has_bazzulto_permissions_section(elf_data: &[u8]) -> bool {
    // ELF64 header is 64 bytes.
    if elf_data.len() < 64 {
        return false;
    }

    // Read e_shoff (offset to section header table) at byte 40.
    let e_shoff = u64::from_le_bytes(
        elf_data[40..48].try_into().unwrap_or([0; 8])
    ) as usize;
    // Read e_shentsize at byte 58, e_shnum at 60, e_shstrndx at 62.
    let e_shentsize = u16::from_le_bytes(
        elf_data[58..60].try_into().unwrap_or([0; 2])
    ) as usize;
    let e_shnum = u16::from_le_bytes(
        elf_data[60..62].try_into().unwrap_or([0; 2])
    ) as usize;
    let e_shstrndx = u16::from_le_bytes(
        elf_data[62..64].try_into().unwrap_or([0; 2])
    ) as usize;

    if e_shoff == 0 || e_shnum == 0 || e_shentsize < 64 {
        return false;
    }

    // Locate the section-name string table (shstrtab) header.
    // Section header layout: sh_name (4), sh_type (4), sh_flags (8),
    //   sh_addr (8), sh_offset (8), sh_size (8), …  total 64 bytes.
    let shstrtab_hdr_off = e_shoff + e_shstrndx * e_shentsize;
    if shstrtab_hdr_off + 64 > elf_data.len() {
        return false;
    }
    let shstrtab_offset = u64::from_le_bytes(
        elf_data[shstrtab_hdr_off + 24 .. shstrtab_hdr_off + 32]
            .try_into().unwrap_or([0; 8])
    ) as usize;
    let shstrtab_size = u64::from_le_bytes(
        elf_data[shstrtab_hdr_off + 32 .. shstrtab_hdr_off + 40]
            .try_into().unwrap_or([0; 8])
    ) as usize;

    if shstrtab_offset == 0
        || shstrtab_offset + shstrtab_size > elf_data.len()
        || shstrtab_size == 0
    {
        return false;
    }
    let shstrtab = &elf_data[shstrtab_offset .. shstrtab_offset + shstrtab_size];

    // Walk section headers and look for `.bazzulto_permissions`.
    for section_index in 0..e_shnum {
        let hdr_off = e_shoff + section_index * e_shentsize;
        if hdr_off + 4 > elf_data.len() {
            break;
        }
        // sh_name is the first 4-byte field: byte offset into shstrtab.
        let sh_name = u32::from_le_bytes(
            elf_data[hdr_off .. hdr_off + 4].try_into().unwrap_or([0; 4])
        ) as usize;
        if sh_name >= shstrtab_size {
            continue;
        }
        // Find the NUL-terminated name in shstrtab.
        let name_bytes = &shstrtab[sh_name..];
        let name_end = name_bytes.iter().position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        let name = &name_bytes[..name_end];
        if name == b".bazzulto_permissions" {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tier-dispatch helpers used by sys_exec
// ---------------------------------------------------------------------------

/// Resolve the trust tier for a binary at `canonical_path` and return the
/// `(granted_permissions, granted_actions)` set to install on the new image.
///
/// Tier 1 (system binary path) → wildcard set, full action set.
/// Tier 4 (no section present) → inherit from `parent_permissions`.
///
/// `has_permissions_section` must be the result of
/// `elf_has_bazzulto_permissions_section` for this binary.  When `true`, the
/// function returns `None` — the caller should *not* replace the process's
/// permission sets; permissiond will populate them in post-v1.0.
///
/// Returns `Some((access_permissions, action_permissions))` when the kernel
/// can determine the tier autonomously, or `None` when permissiond is needed
/// (Tier 2/3).
pub fn resolve_exec_permissions(
    canonical_path: &str,
    has_permissions_section: bool,
    parent_permissions: &[PathPattern],
    parent_actions: &[ActionPermission],
    binary_name: &str,       // for Tier-4 warning
) -> Option<(Vec<PathPattern>, Vec<ActionPermission>)> {
    // Tier 1: system binary — unconditional full trust.
    // The Ed25519 signature check is deferred to post-v1.0.
    if is_system_binary_path(canonical_path) {
        return Some((
            alloc::vec![PathPattern::wildcard()],
            alloc::vec![
                ActionPermission::MountFilesystem,
                ActionPermission::LoadKernelDriver,
                ActionPermission::ModifyNetworkConfig,
                ActionPermission::InstallPackage,
                ActionPermission::ModifyKernelParams,
                // GrantPermissions is intentionally excluded.
            ],
        ));
    }

    // Tier 2/3: binary has a .bazzulto_permissions section.
    // permissiond must interpret it.  Return None — do not touch the sets.
    if has_permissions_section {
        return None;
    }

    // Tier 4: no declaration.  Inherit from parent and warn.
    emit_tier4_warning(binary_name);
    Some((
        parent_permissions.to_vec(),
        parent_actions.to_vec(),
    ))
}

/// Return `true` if `path` is a Tier-1 system binary path.
pub fn is_system_binary_path(path: &str) -> bool {
    path.starts_with("//system:/bin/")
        || path.starts_with("//system:/sbin/")
        || path.starts_with("/system/bin/")
        || path.starts_with("/system/sbin/")
}

/// Write a Tier-4 warning to the kernel log (not to the process's stderr —
/// the process does not have a TTY connection at exec time in this kernel).
///
/// In a future release, this warning will be forwarded to the process's
/// stderr via a permissiond message.
fn emit_tier4_warning(binary_name: &str) {
    crate::drivers::uart::puts("[bazzulto] warning: ");
    crate::drivers::uart::puts(binary_name);
    crate::drivers::uart::puts(" has no permission declaration — running with inherited permissions\r\n");
}
