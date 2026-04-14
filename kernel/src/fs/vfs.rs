// fs/vfs.rs — Virtual File System layer.
//
// Provides a per-process file descriptor table and abstracts over concrete
// file types (ramfs files, pipes, TTY).
//
// Design:
//   - FileDescriptor enum with variants for each concrete type.
//   - Per-process FD table (MAX_OPEN_FILE_DESCRIPTORS = 1024 slots).
//   - The FD table is owned by the Process and cloned on fork().
//
// Reference:
//   Linux fs/file.c (struct files_struct, fdt).
//   POSIX.1-2017 §2.14 (file descriptor table).

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::pipe::{PipeHandle, PipeEnd, pipe_read_blocking, pipe_write_blocking};
use super::procfs::ProcSnapshot;
use super::inode::Inode;

/// Maximum open file descriptors per process.
///
/// Reference: Linux default RLIMIT_NOFILE soft = 1024.
pub use crate::process::MAX_OPEN_FILE_DESCRIPTORS;

// ---------------------------------------------------------------------------
// FileDescriptor
// ---------------------------------------------------------------------------

/// A concrete open file.
pub enum FileDescriptor {
    /// A file in the read-only ramfs.
    RamFsFile {
        /// Static byte slice — the file's data.
        data: &'static [u8],
        /// Current byte position within the file.
        position: usize,
    },
    /// One end of a kernel pipe.
    Pipe(PipeHandle),
    /// The TTY (standard input/output terminal).
    Tty,
    /// A procfs virtual file snapshot.
    ProcFile(ProcSnapshot),

    /// A VFS inode-backed file (tmpfs, devfs, or any Inode implementation).
    ///
    /// `position` is the current byte offset for sequential reads/writes.
    InoFile {
        inode: Arc<dyn Inode>,
        /// Current read/write position.
        position: u64,
    },
}

impl FileDescriptor {
    /// Clone this descriptor for use by dup() / fork().
    ///
    /// For pipes, increments the appropriate reference count.
    /// For RamFsFile, the slice pointer is duplicated.
    pub fn dup(&self) -> Option<FileDescriptor> {
        match self {
            FileDescriptor::RamFsFile { data, position } => {
                Some(FileDescriptor::RamFsFile { data, position: *position })
            }
            FileDescriptor::Pipe(handle) => {
                let cloned = match handle.end() {
                    PipeEnd::ReadEnd => PipeHandle::clone_read_end(handle),
                    PipeEnd::WriteEnd => PipeHandle::clone_write_end(handle),
                };
                Some(FileDescriptor::Pipe(cloned))
            }
            FileDescriptor::Tty => Some(FileDescriptor::Tty),
            FileDescriptor::ProcFile(snapshot) => {
                Some(FileDescriptor::ProcFile(ProcSnapshot {
                    data: snapshot.data.clone(),
                    position: snapshot.position,
                }))
            }
            FileDescriptor::InoFile { inode, position } => {
                Some(FileDescriptor::InoFile {
                    inode: inode.clone(),
                    position: *position,
                })
            }
        }
    }

    /// Read up to `destination.len()` bytes.
    ///
    /// # Safety
    /// Must be called with IRQs disabled when the descriptor is a Pipe
    /// (may block via the scheduler).
    pub unsafe fn read(&mut self, destination: &mut [u8]) -> i64 {
        match self {
            FileDescriptor::RamFsFile { data, position } => {
                let available = data.len().saturating_sub(*position);
                let to_read = destination.len().min(available);
                destination[..to_read].copy_from_slice(&data[*position..*position + to_read]);
                *position += to_read;
                to_read as i64
            }
            FileDescriptor::Pipe(handle) => {
                if handle.end() != PipeEnd::ReadEnd {
                    return -9; // EBADF — writing end used for read
                }
                pipe_read_blocking(handle, destination) as i64
            }
            FileDescriptor::Tty => {
                // Read a line from the TTY.
                crate::drivers::tty::tty_read_bytes(destination) as i64
            }
            FileDescriptor::ProcFile(snapshot) => {
                snapshot.read(destination) as i64
            }
            FileDescriptor::InoFile { inode, position } => {
                match inode.read_at(*position, destination) {
                    Ok(n) => {
                        *position += n as u64;
                        n as i64
                    }
                    Err(error) => error.to_errno(),
                }
            }
        }
    }

    /// Write `source` bytes.
    ///
    /// # Safety
    /// Must be called with IRQs disabled when the descriptor is a Pipe.
    pub unsafe fn write(&mut self, source: &[u8]) -> i64 {
        match self {
            FileDescriptor::RamFsFile { .. } => {
                -9 // EBADF — read-only file
            }
            FileDescriptor::Pipe(handle) => {
                if handle.end() != PipeEnd::WriteEnd {
                    return -9; // EBADF
                }
                match pipe_write_blocking(handle, source) {
                    crate::fs::pipe::PipeWriteResult::Written(n) => n as i64,
                    // Returning a dedicated sentinel so the syscall layer can
                    // send SIGPIPE before returning EPIPE to userspace.
                    // POSIX.1-2017 write(2): SIGPIPE is generated; errno = EPIPE.
                    crate::fs::pipe::PipeWriteResult::BrokenPipe(_) => i64::MIN,
                }
            }
            FileDescriptor::Tty => {
                for &byte in source {
                    crate::drivers::uart::putc(byte);
                    crate::drivers::console::print_char(byte as char);
                }
                source.len() as i64
            }
            FileDescriptor::ProcFile(_) => {
                -9 // EBADF — read-only
            }
            FileDescriptor::InoFile { inode, position } => {
                match inode.write_at(*position, source) {
                    Ok(n) => {
                        *position += n as u64;
                        n as i64
                    }
                    Err(error) => error.to_errno(),
                }
            }
        }
    }

    /// Seek within the file.  Returns new position or -1 on error.
    ///
    /// Only meaningful for RamFsFile; pipes and TTY return -1 (ESPIPE).
    pub fn seek(&mut self, offset: i64, whence: i32) -> i64 {
        match self {
            FileDescriptor::RamFsFile { data, position } => {
                let file_length = data.len() as i64;
                let new_position: i64 = match whence {
                    0 => offset,               // SEEK_SET
                    1 => *position as i64 + offset, // SEEK_CUR
                    2 => file_length + offset,  // SEEK_END
                    _ => return -22,            // EINVAL
                };
                if new_position < 0 || new_position > file_length {
                    return -22; // EINVAL
                }
                *position = new_position as usize;
                new_position as i64
            }
            FileDescriptor::InoFile { inode, position } => {
                let file_size = inode.stat().size as i64;
                let new_pos: i64 = match whence {
                    0 => offset,
                    1 => *position as i64 + offset,
                    2 => file_size + offset,
                    _ => return -22,
                };
                if new_pos < 0 {
                    return -22;
                }
                *position = new_pos as u64;
                new_pos
            }
            _ => -29, // ESPIPE — not seekable
        }
    }
}

// ---------------------------------------------------------------------------
// FileDescriptorTable
// ---------------------------------------------------------------------------

/// Per-process file descriptor table.
///
/// Slot 0 = stdin (TTY), slot 1 = stdout (TTY), slot 2 = stderr (TTY)
/// by convention.  The table is initialised with TTY descriptors in those
/// three slots and None in the rest.
///
/// Stored as a heap-allocated Vec of Options to avoid a large fixed-size array
/// on the kernel stack.
///
/// Shared between all threads in a thread group via `Arc<SpinLock<FileDescriptorTable>>`.
/// POSIX requires all threads in a group to share the same file descriptor table.
pub struct FileDescriptorTable {
    slots: Vec<Option<FileDescriptor>>,
    /// Bitmask of file descriptors with O_CLOEXEC set (bit N = fd N).
    ///
    /// Stored as 16 × u64 words to cover up to 1024 file descriptors without
    /// aliasing.  Word index = fd / 64, bit index within word = fd % 64.
    /// Reference: POSIX.1-2017 fcntl(2) FD_CLOEXEC.
    pub cloexec_mask: [u64; 16],
    /// Bitmask of file descriptors with O_NONBLOCK set (bit N = fd N).
    ///
    /// Same layout as `cloexec_mask`.
    pub nonblock_mask: [u64; 16],
}

impl FileDescriptorTable {
    /// Create a new table with stdin/stdout/stderr wired to the TTY.
    pub fn new_with_tty() -> Self {
        let mut slots = Vec::with_capacity(8);
        slots.push(Some(FileDescriptor::Tty)); // fd 0: stdin
        slots.push(Some(FileDescriptor::Tty)); // fd 1: stdout
        slots.push(Some(FileDescriptor::Tty)); // fd 2: stderr
        Self { slots, cloexec_mask: [0u64; 16], nonblock_mask: [0u64; 16] }
    }

    /// Create an empty table (all None).
    pub fn empty() -> Self {
        Self { slots: Vec::new(), cloexec_mask: [0u64; 16], nonblock_mask: [0u64; 16] }
    }

    /// Deep-clone all descriptors for fork().
    ///
    /// The child receives an independent copy of the parent's FD table with
    /// duplicated pipe reference counts.  POSIX fork() semantics.
    pub fn clone_for_fork(&self) -> Self {
        let mut new_slots = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            new_slots.push(slot.as_ref().and_then(|fd| fd.dup()));
        }
        Self { slots: new_slots, cloexec_mask: self.cloexec_mask, nonblock_mask: self.nonblock_mask  }
    }

    /// Allocate the lowest free file descriptor and install `descriptor`.
    ///
    /// Returns the new file descriptor number or -1 if the table is full.
    pub fn install(&mut self, descriptor: FileDescriptor) -> i32 {
        // Search for an existing None slot first.
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(descriptor);
                return index as i32;
            }
        }
        // No free slot found; grow the table if under the limit.
        if self.slots.len() >= MAX_OPEN_FILE_DESCRIPTORS {
            return -1;
        }
        let fd = self.slots.len() as i32;
        self.slots.push(Some(descriptor));
        fd
    }

    /// Install `descriptor` at a specific file descriptor number.
    ///
    /// If a descriptor is already present at `fd`, it is closed first.
    /// Returns `fd` on success or -1 if `fd` is out of range.
    pub fn install_at(&mut self, fd: usize, descriptor: FileDescriptor) -> i32 {
        if fd >= MAX_OPEN_FILE_DESCRIPTORS {
            return -1;
        }
        // Grow the table if needed.
        while self.slots.len() <= fd {
            self.slots.push(None);
        }
        self.slots[fd] = Some(descriptor);
        fd as i32
    }

    /// Get a shared reference to the descriptor at `fd`, or `None` if closed/out of range.
    pub fn get(&self, fd: usize) -> Option<&FileDescriptor> {
        self.slots.get(fd)?.as_ref()
    }

    /// Close the file descriptor `fd`, dropping the descriptor.
    ///
    /// Returns true if closed, false if fd was already None or out of range.
    pub fn close(&mut self, fd: usize) -> bool {
        match self.slots.get_mut(fd) {
            Some(slot) if slot.is_some() => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    /// Return the word index and bit mask for `fd` within a 1024-bit mask array.
    ///
    /// `fd` must be < MAX_OPEN_FILE_DESCRIPTORS (1024).
    /// Returns `(word_index, bit_mask)` where `mask[word_index] & bit_mask` tests the fd.
    #[inline]
    fn fd_mask_pos(fd: usize) -> (usize, u64) {
        let word = fd / 64;
        let bit  = 1u64 << (fd % 64);
        (word, bit)
    }

    /// Set a flag bit for `fd` in `mask`.
    #[inline]
    pub fn mask_set(mask: &mut [u64; 16], fd: usize) {
        let (word, bit) = Self::fd_mask_pos(fd);
        mask[word] |= bit;
    }

    /// Clear a flag bit for `fd` in `mask`.
    #[inline]
    pub fn mask_clear(mask: &mut [u64; 16], fd: usize) {
        let (word, bit) = Self::fd_mask_pos(fd);
        mask[word] &= !bit;
    }

    /// Test a flag bit for `fd` in `mask`.
    #[inline]
    pub fn mask_test(mask: &[u64; 16], fd: usize) -> bool {
        let (word, bit) = Self::fd_mask_pos(fd);
        mask[word] & bit != 0
    }

    /// Close all open file descriptors (called on process exit).
    pub fn close_all(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
    }

    /// Get a mutable reference to an open descriptor.
    pub fn get_mut(&mut self, fd: usize) -> Option<&mut FileDescriptor> {
        self.slots.get_mut(fd)?.as_mut()
    }

    /// Duplicate `source_fd` into `destination_fd` (dup2 semantics).
    ///
    /// If `source_fd == destination_fd` and source is open, returns success
    /// without doing anything.
    ///
    /// Returns `destination_fd` on success, negative errno on failure.
    pub fn dup2(&mut self, source_fd: usize, destination_fd: usize) -> i32 {
        if source_fd >= self.slots.len() || self.slots[source_fd].is_none() {
            return -9; // EBADF
        }
        if source_fd == destination_fd {
            return destination_fd as i32;
        }
        let new_descriptor = match &self.slots[source_fd] {
            Some(descriptor) => descriptor.dup(),
            None => return -9,
        };
        let new_descriptor = match new_descriptor {
            Some(descriptor) => descriptor,
            None => return -9,
        };
        let result = self.install_at(destination_fd, new_descriptor);
        if result >= 0 {
            // POSIX.1-2017 dup2(2): "The FD_CLOEXEC flag associated with the
            // new file descriptor shall be cleared to keep the file descriptor
            // open across calls to one of the exec functions."
            // O_NONBLOCK is inherited from the source fd's nonblock_mask.
            Self::mask_clear(&mut self.cloexec_mask, destination_fd);
        }
        result
    }

    /// Duplicate `source_fd` into the lowest available slot (dup semantics).
    ///
    /// Returns the new file descriptor number or negative errno on failure.
    pub fn dup(&mut self, source_fd: usize) -> i32 {
        let new_descriptor = match self.slots.get(source_fd) {
            Some(Some(descriptor)) => match descriptor.dup() {
                Some(new_descriptor) => new_descriptor,
                None => return -9,
            },
            _ => return -9, // EBADF
        };
        self.install(new_descriptor)
    }
}
