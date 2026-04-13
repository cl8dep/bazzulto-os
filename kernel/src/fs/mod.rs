// fs/mod.rs — File system modules.

pub mod devfs;
pub mod epoll;
pub mod fat32;
pub mod fifo;
pub mod inode;
pub mod mount;
pub mod partition;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod tmpfs;
pub mod vfs;

pub use vfs::FileDescriptor;
pub use ramfs::{ramfs_find, ramfs_list, ramfs_register_file};
pub use mount::{vfs_init, vfs_mount, vfs_resolve, vfs_resolve_parent, with_vfs};
pub use inode::{Inode, InodeType, InodeStat, DirEntry, FsError};
