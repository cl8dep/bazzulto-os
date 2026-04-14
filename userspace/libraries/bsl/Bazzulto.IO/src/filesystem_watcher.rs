//! Filesystem change notification — forward-declaration stub.
//!
//! Corresponds to `System.IO.FileSystemWatcher` in the .NET BCL.
//!
//! # Status
//!
//! NOT IMPLEMENTED. Bazzulto OS does not yet have filesystem change
//! notification syscalls (inotify equivalent). This type is present as a
//! forward-declaration stub so API consumers can be written now and activated
//! once the kernel adds the required syscalls.
//!
//! # Required kernel additions
//!
//! ```text
//! TODO: sys_inotify_init() → fd
//!     Creates a new inotify instance. Returns a file descriptor that the
//!     caller reads to receive inotify_event structs via raw_read().
//!
//! TODO: sys_inotify_add_watch(fd, path_ptr, path_len, mask) → watch_descriptor
//!     Adds or modifies a watch for the filesystem object at path.
//!     mask is a bitmask of IN_CREATE | IN_DELETE | IN_MODIFY | IN_MOVED_FROM
//!     | IN_MOVED_TO (matching Linux inotify(7) values).
//!
//! TODO: sys_inotify_rm_watch(fd, wd) → i32
//!     Removes the watch identified by wd from the inotify instance fd.
//! ```
//!
//! Once those vDSO slots are wired, this stub can be replaced with a real
//! implementation that:
//! 1. Calls `sys_inotify_init()` in `FileSystemWatcher::new`.
//! 2. Calls `sys_inotify_add_watch` with the caller-supplied path and mask.
//! 3. Exposes a `next_event() -> Option<FilesystemEvent>` method that calls
//!    `raw_read` on the inotify fd and parses `inotify_event` structs.
//! 4. Implements `Drop` to close the inotify fd.

extern crate alloc;

use alloc::string::String;

// ---------------------------------------------------------------------------
// FileSystemWatcher (stub)
// ---------------------------------------------------------------------------

/// Watches a directory for filesystem changes (create, delete, modify, rename).
///
/// This is a forward-declaration stub. All methods return `Err(-38)` (ENOSYS)
/// until the kernel exposes inotify-equivalent syscalls.
pub struct FileSystemWatcher {
    /// Path that would be watched once implemented.
    _path: String,
}

impl FileSystemWatcher {
    /// Create a watcher for the directory at `path`.
    ///
    /// Always returns `Err(-38)` (ENOSYS) because the required kernel
    /// syscalls do not exist yet. See module-level documentation.
    pub fn new(_path: &str) -> Result<Self, i32> {
        Err(-38) // ENOSYS
    }
}
