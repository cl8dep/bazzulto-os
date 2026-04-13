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
        let result = raw::raw_mkdir(path.as_ptr(), path.len(), 0o755);
        if result < 0 {
            let _ = error_output.write_all(b"bzinit: warn: POSIX compat target '");
            let _ = error_output.write_all(path.as_bytes());
            let _ = error_output.write_all(b"' does not exist -- translations to it will fail\n");
        }
    }
}

/// Return true if a path can be opened (proxy for existence check).
fn path_exists(path: &str) -> bool {
    let fd = raw::raw_open(path.as_ptr(), path.len());
    if fd >= 0 {
        raw::raw_close(fd as i32);
        true
    } else {
        false
    }
}
