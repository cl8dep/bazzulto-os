//! Path helpers — normalize and join Bazzulto paths.
//!
//! Both the `//scheme:authority/rest` form and standard Unix paths are valid.
//! This module does not perform kernel I/O; it only manipulates strings.

use alloc::string::String;
use alloc::vec::Vec;

/// A heap-allocated absolute path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path(String);

impl Path {
    /// Wrap a string that is already an absolute path.
    pub fn new(s: &str) -> Self {
        Path(String::from(s))
    }

    /// Return the path as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the file name component (everything after the last `/`).
    /// Returns the full string if there is no `/`.
    pub fn file_name(&self) -> &str {
        match self.0.rfind('/') {
            Some(pos) => &self.0[pos + 1..],
            None => &self.0,
        }
    }

    /// Return the parent directory path, or `None` if already at root.
    pub fn parent(&self) -> Option<Path> {
        let s = self.0.trim_end_matches('/');
        let pos = s.rfind('/')?;
        if pos == 0 {
            Some(Path(String::from("/")))
        } else {
            Some(Path(String::from(&s[..pos])))
        }
    }

    /// Append a path component.
    pub fn join(&self, component: &str) -> Path {
        let mut result = self.0.clone();
        if !result.ends_with('/') {
            result.push('/');
        }
        result.push_str(component.trim_start_matches('/'));
        Path(result)
    }

    /// Return the extension (everything after the last `.` in the file name),
    /// or `None` if there is no extension.
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        let dot = name.rfind('.')?;
        if dot == 0 {
            None // dotfiles like ".config" have no extension
        } else {
            Some(&name[dot + 1..])
        }
    }

    /// True if this path ends with the given suffix (e.g. `.service`).
    pub fn ends_with(&self, suffix: &str) -> bool {
        self.0.ends_with(suffix)
    }
}

impl core::fmt::Display for Path {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.0)
    }
}
