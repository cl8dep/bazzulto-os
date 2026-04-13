// fs/mount.rs — VFS mount table and path resolution.
//
// The mount table maps path prefixes to root inodes of mounted filesystems.
// Path resolution traverses the table to find the deepest matching mount point,
// then walks the remaining components through `Inode::lookup()`.
//
// Mount table layout (ordered by decreasing prefix length for longest-match):
//   "/"      → tmpfs root (always present)
//   "/dev"   → devfs root
//   "/proc"  → procfs root (virtual)
//   "/tmp"   → tmpfs (shared with root or separate instance)
//
// Path resolution rules:
//   - Absolute paths: start from the matching mount point.
//   - Relative paths: start from the process's cwd.
//   - "." is skipped.
//   - ".." climbs to the parent (within the mount point).
//
// Reference: Linux fs/namei.c `path_lookup()`, `link_path_walk()`.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use super::inode::{FsError, Inode, InodeType};

// ---------------------------------------------------------------------------
// MountEntry
// ---------------------------------------------------------------------------

struct MountEntry {
    /// Absolute path prefix (e.g. "/", "/dev", "/proc").
    prefix: String,
    /// Root inode of the mounted filesystem.
    root: Arc<dyn Inode>,
}

// ---------------------------------------------------------------------------
// MountTable
// ---------------------------------------------------------------------------

pub struct MountTable {
    entries: Vec<MountEntry>,
}

// SAFETY: single-core, IRQs disabled during all accesses.
unsafe impl Send for MountTable {}
unsafe impl Sync for MountTable {}

impl MountTable {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Mount `root` at `path`.
    ///
    /// Longer paths take precedence over shorter ones (longest-match).
    /// Re-mounting an existing path replaces the previous entry.
    pub fn mount(&mut self, path: &str, root: Arc<dyn Inode>) {
        // Remove any existing entry for this path.
        self.entries.retain(|entry| entry.prefix != path);
        self.entries.push(MountEntry {
            prefix: path.to_string(),
            root,
        });
        // Sort longest-prefix first so resolve() finds the best match quickly.
        self.entries.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
    }

    /// Resolve an absolute path to its inode.
    ///
    /// Returns `(parent_inode, file_name)` where `parent_inode` is the
    /// directory that directly contains the resolved inode, and `file_name`
    /// is the final path component.
    ///
    /// Use `resolve_inode()` for a simpler interface when you only need the
    /// inode itself.
    pub fn resolve_parent(&self, path: &str) -> Result<(Arc<dyn Inode>, String), FsError> {
        let (mut current, remaining) = self.find_mount_root(path)?;

        // Split remaining path into components.
        let components: Vec<&str> = remaining
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect();

        if components.is_empty() {
            // Path resolves to the mount root itself — no parent in this mount.
            // Return (root, ".") as a special case for the root directory.
            return Ok((current, ".".to_string()));
        }

        // Walk all components except the last.
        for component in &components[..components.len() - 1] {
            if *component == ".." {
                // ".." at mount root stays at root (simplified — no cross-mount "..")
                // A full implementation would track the mount tree.
                continue;
            }
            if current.inode_type() != InodeType::Directory {
                return Err(FsError::NotDirectory);
            }
            current = current.lookup(component).ok_or(FsError::NotFound)?;
        }

        let last_component = components[components.len() - 1].to_string();
        Ok((current, last_component))
    }

    /// Resolve a path to its inode.
    ///
    /// `cwd` is used for relative paths.
    /// For absolute paths, `cwd` is ignored and the mount table is used.
    pub fn resolve_inode(
        &self,
        path: &str,
        cwd: Option<&Arc<dyn Inode>>,
    ) -> Result<Arc<dyn Inode>, FsError> {
        if path.starts_with('/') {
            // Absolute path.
            let (parent, name) = self.resolve_parent(path)?;
            if name == "." {
                return Ok(parent); // resolved to mount root
            }
            parent.lookup(&name).ok_or(FsError::NotFound)
        } else {
            // Relative path: start from cwd.
            let start = cwd.cloned().ok_or(FsError::NotFound)?;
            resolve_relative(start, path)
        }
    }

    /// Find the mount entry with the longest matching prefix of `path`.
    ///
    /// Returns `(root_inode, remaining_path_after_prefix)`.
    fn find_mount_root<'a>(&self, path: &'a str) -> Result<(Arc<dyn Inode>, &'a str), FsError> {
        for entry in &self.entries {
            let prefix = entry.prefix.as_str();
            if path == prefix {
                return Ok((entry.root.clone(), ""));
            }
            if path.starts_with(prefix) {
                // Prefix must end at a component boundary (either "/" or end of prefix).
                let rest = &path[prefix.len()..];
                if rest.starts_with('/') || prefix == "/" {
                    let trimmed = if prefix == "/" { path } else { rest };
                    return Ok((entry.root.clone(), trimmed));
                }
            }
        }
        Err(FsError::NotFound)
    }
}

/// Resolve a relative path starting from `start`.
fn resolve_relative(mut current: Arc<dyn Inode>, path: &str) -> Result<Arc<dyn Inode>, FsError> {
    for component in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
        if component == ".." {
            // ".." — simplified: stay at current for now.
            // A full implementation needs parent tracking.
            continue;
        }
        if current.inode_type() != InodeType::Directory {
            return Err(FsError::NotDirectory);
        }
        let next = current.lookup(component).ok_or(FsError::NotFound)?;
        // Follow symbolic links transparently during path traversal.
        current = follow_symlinks(next, 0)?;
    }
    Ok(current)
}

/// Follow a chain of symbolic links up to `MAX_SYMLINK_DEPTH` hops.
///
/// Returns the final non-symlink inode, or `FsError::TooManyLinks` if the
/// chain exceeds the depth limit.
///
/// Reference: POSIX.1-2017 §2.3 (Symbolic Links) — limit of 8 levels.
const MAX_SYMLINK_DEPTH: usize = 8;

fn follow_symlinks(
    inode: alloc::sync::Arc<dyn super::inode::Inode>,
    depth: usize,
) -> Result<alloc::sync::Arc<dyn super::inode::Inode>, super::inode::FsError> {
    use super::inode::InodeType;
    if inode.inode_type() != InodeType::Symlink {
        return Ok(inode);
    }
    if depth >= MAX_SYMLINK_DEPTH {
        return Err(super::inode::FsError::TooManyLinks);
    }
    // Read the target path from the symlink inode.
    let mut buf = [0u8; 512];
    let len = inode.read_at(0, &mut buf)?;
    let target = core::str::from_utf8(&buf[..len])
        .map_err(|_| super::inode::FsError::InvalidArgument)?;
    // Resolve target through the VFS (symlink targets in this kernel are
    // always absolute — procfs /proc/self → /proc/<pid>).
    let resolved = unsafe { vfs_resolve(target, None)? };
    follow_symlinks(resolved, depth + 1)
}

// ---------------------------------------------------------------------------
// Global VFS mount table
// ---------------------------------------------------------------------------

struct SyncMountTable(UnsafeCell<Option<MountTable>>);
unsafe impl Sync for SyncMountTable {}

static VFS_MOUNT_TABLE: SyncMountTable = SyncMountTable(UnsafeCell::new(None));

/// Initialise the global VFS with a tmpfs root and devfs at /dev.
///
/// Must be called once during kernel boot after the heap is available.
///
/// # Safety
/// Must be called single-threaded with IRQs disabled.
pub unsafe fn vfs_init() {
    use super::tmpfs::TmpfsDir;
    use super::devfs::{devfs_create, devfs_seed_entropy};

    let table_slot = &mut *VFS_MOUNT_TABLE.0.get();

    let mut table = MountTable::new();

    // Root filesystem: tmpfs at "/".
    let root = TmpfsDir::new();

    // Create standard top-level directories.
    let _ = root.mkdir("tmp");
    let _ = root.mkdir("mnt");
    let _ = root.mkdir("bin");
    let _ = root.mkdir("etc");
    root.insert("dev", devfs_create()); // /dev

    table.mount("/", root);

    // Mount the virtual procfs at "/proc".
    table.mount("/proc", super::procfs::ProcfsRootInode::new());

    // Seed entropy from CNTPCT_EL0 (available after Phase 3).
    let cntpct: u64;
    core::arch::asm!("mrs {}, cntpct_el0", out(reg) cntpct);
    devfs_seed_entropy(cntpct);

    *table_slot = Some(table);
}

/// Access the global VFS mount table.
///
/// # Safety
/// Must be called after `vfs_init()` and with IRQs disabled.
pub unsafe fn with_vfs<F, R>(function: F) -> R
where
    F: FnOnce(&mut MountTable) -> R,
{
    let slot = &mut *VFS_MOUNT_TABLE.0.get();
    function(slot.as_mut().expect("vfs not initialised"))
}

/// Mount an additional filesystem at `path`.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn vfs_mount(path: &str, root: Arc<dyn Inode>) {
    with_vfs(|table| table.mount(path, root));
}

/// Resolve an absolute or relative path to an inode.
///
/// `cwd` is used for relative paths.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn vfs_resolve(
    path: &str,
    cwd: Option<&Arc<dyn Inode>>,
) -> Result<Arc<dyn Inode>, FsError> {
    with_vfs(|table| table.resolve_inode(path, cwd))
}

/// Resolve a path and return the (parent directory, file name) pair.
///
/// Used by `open(O_CREAT)`, `mkdir`, `unlink`, `rename`.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn vfs_resolve_parent(path: &str) -> Result<(Arc<dyn Inode>, String), FsError> {
    with_vfs(|table| table.resolve_parent(path))
}
