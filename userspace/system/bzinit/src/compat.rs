//! Boot-time POSIX compatibility setup.
//!
//! Creates the directories required by the POSIX path translation table if
//! they don't already exist. Logs a warning for any directory that cannot be
//! created (e.g. if the parent doesn't exist yet).
//!
//! Directory creation order matters: parents must be created before children.

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
    "/etc",
    "/data",
    "/data/temp",
    "/data/logs",
    "/dev",
    "/proc",
    "/apps",
];

/// Ensure all POSIX compat translation targets exist.
///
/// Creates missing directories via mkdir. Already-existing directories are
/// silently skipped. Logs a warning to stderr only if mkdir fails for a
/// reason other than the directory already existing.
pub fn validate_compat_targets() {
    let error_output = stderr();
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
