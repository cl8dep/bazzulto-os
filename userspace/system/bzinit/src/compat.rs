//! Boot-time POSIX compatibility setup.
//!
//! Creates the directories required by the POSIX path translation table if
//! they don't already exist, then creates compatibility symlinks so that
//! software hardcoding traditional Unix paths still works.
//!
//! Directory creation order matters: parents must be created before children.
//!
//! ALL compatibility path translations are centralised here — do not scatter
//! symlink or directory creation across multiple files.

use bazzulto_system::raw;
use bazzulto_io::stream::stderr;

/// Directories to ensure exist at boot, in creation order (parents first).
///
/// These are the targets of the POSIX path translation table in libc_compat.
const REQUIRED_DIRS: &[&str] = &[
    "/home",
    "/home/user",
    "/home/user/.bin",
    "/home/user/.lib",
    "/system",
    "/system/lib",
    "/system/share",
    // IANA timezone database (TZif binary files, same layout as Linux/macOS).
    "/system/share/zoneinfo",
    "/system/share/zoneinfo/Africa",
    "/system/share/zoneinfo/America",
    "/system/share/zoneinfo/America/Argentina",
    "/system/share/zoneinfo/America/Indiana",
    "/system/share/zoneinfo/America/Kentucky",
    "/system/share/zoneinfo/America/North_Dakota",
    "/system/share/zoneinfo/Antarctica",
    "/system/share/zoneinfo/Arctic",
    "/system/share/zoneinfo/Asia",
    "/system/share/zoneinfo/Atlantic",
    "/system/share/zoneinfo/Australia",
    "/system/share/zoneinfo/Brazil",
    "/system/share/zoneinfo/Canada",
    "/system/share/zoneinfo/Chile",
    "/system/share/zoneinfo/Etc",
    "/system/share/zoneinfo/Europe",
    "/system/share/zoneinfo/Indian",
    "/system/share/zoneinfo/Mexico",
    "/system/share/zoneinfo/Pacific",
    "/system/share/zoneinfo/US",
    "/data",
    "/data/temp",
    "/data/logs",
    "/dev",
    "/proc",
    "/apps",
];

/// POSIX compatibility symlinks — traditional Unix paths → Bazzulto equivalents.
///
/// Bazzulto uses `/system/` as the system root instead of the traditional Unix
/// FHS layout.  These symlinks ensure that software hardcoding traditional
/// paths (e.g. `/etc/passwd`, `/bin/sh`) still resolves correctly.
///
/// Format: `(link_path, target_path)`
///   - `link_path`:   the traditional Unix path (created as a symlink)
///   - `target_path`: the real Bazzulto path it points to
///
/// **To add a new compat symlink, add ONE line here.**  Do not scatter symlink
/// creation across multiple files — this is the single source of truth for all
/// path compatibility translations.
///
/// Reference: docs/Roadmap.md M3 §3.7, Bazzulto Path Model.
const COMPAT_SYMLINKS: &[(&str, &str)] = &[
    // /etc → /system/config: user database (passwd, group, shadow, hostname),
    // fstab, mtab, shells, and other POSIX configuration files.
    ("/etc", "/system/config"),
];

/// Ensure all POSIX compat directories exist and symlinks are created.
///
/// Creates missing directories via mkdir. Already-existing directories are
/// silently skipped. Then creates each symlink in `COMPAT_SYMLINKS`.
/// Logs a warning to stderr for any operation that fails.
pub fn validate_compat_targets() {
    let error_output = stderr();

    // Phase 1: create required directories.
    for path in REQUIRED_DIRS {
        if path_exists(path) {
            continue;
        }
        let mut mkdir_path_buf = [0u8; 512];
        let mkdir_path_len = path.len().min(511);
        mkdir_path_buf[..mkdir_path_len].copy_from_slice(&path.as_bytes()[..mkdir_path_len]);
        let result = raw::raw_mkdir(mkdir_path_buf.as_ptr(), 0o755);
        if result < 0 {
            let _ = error_output.write_all(b"bzinit: warn: POSIX compat target '");
            let _ = error_output.write_all(path.as_bytes());
            let _ = error_output.write_all(b"' does not exist -- translations to it will fail\n");
        }
    }

    // Phase 2: create compatibility symlinks.
    for &(link_path, target_path) in COMPAT_SYMLINKS {
        if path_exists(link_path) {
            continue; // Already exists (directory, file, or symlink) — skip.
        }
        let mut target_buf = [0u8; 512];
        let target_len = target_path.len().min(511);
        target_buf[..target_len].copy_from_slice(&target_path.as_bytes()[..target_len]);

        let mut link_buf = [0u8; 512];
        let link_len = link_path.len().min(511);
        link_buf[..link_len].copy_from_slice(&link_path.as_bytes()[..link_len]);

        let result = raw::raw_symlink(target_buf.as_ptr(), link_buf.as_ptr());
        if result < 0 {
            let _ = error_output.write_all(b"bzinit: warn: compat symlink '");
            let _ = error_output.write_all(link_path.as_bytes());
            let _ = error_output.write_all(b"' -> '");
            let _ = error_output.write_all(target_path.as_bytes());
            let _ = error_output.write_all(b"' failed\n");
        }
    }
}

/// Return true if a path can be opened (proxy for existence check).
fn path_exists(path: &str) -> bool {
    let mut path_buf = [0u8; 512];
    let path_len = path.len().min(511);
    path_buf[..path_len].copy_from_slice(&path.as_bytes()[..path_len]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd >= 0 {
        raw::raw_close(fd as i32);
        true
    } else {
        false
    }
}
