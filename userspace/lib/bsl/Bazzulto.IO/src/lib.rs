//! Bazzulto.IO — typed I/O primitives for Bazzulto userspace.
//!
//! Provides `File`, `Directory`, `Stream` (stdin/stdout/stderr), and `Path`
//! helpers. All syscall access goes through `bazzulto_system::raw`.

#![no_std]

extern crate alloc;

pub mod directory;
pub mod file;
pub mod path;
pub mod stream;

pub use file::File;
pub use directory::Directory;
pub use path::Path;
pub use stream::{stdin, stdout, stderr, Stream};
