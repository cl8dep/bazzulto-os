// ipc/sem.rs — POSIX named semaphores.
//
// Named semaphores are identified by a short name string (e.g. "/mysem").
// They live in a fixed-size global table, not the VFS.
//
// Operations:
//   sem_open    — open or create a named semaphore; returns an fd.
//   sem_close   — close the fd (decrements ref-count; does not destroy).
//   sem_wait    — decrement (blocks if value == 0).
//   sem_trywait — non-blocking decrement (EAGAIN if value == 0).
//   sem_post    — increment; wake one waiter if any.
//   sem_unlink  — mark for destruction when ref-count reaches 0.
//   sem_getvalue — read the current counter value into a user-space pointer.
//
// Access is serialised by IRQ disabling (single-core invariant).
//
// Reference: POSIX.1-2017 §11.2 (Semaphore Functions).

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::cell::UnsafeCell;

use crate::process::Pid;
use crate::fs::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum length of a semaphore name in bytes (including the leading '/').
///
/// Linux uses NAME_MAX (255); we use a compact fixed-size value to keep
/// NamedSemaphore on the stack without heap allocation.
pub const SEMAPHORE_NAME_MAX_LENGTH: usize = 64;

/// Maximum number of named semaphores that can exist simultaneously.
///
/// POSIX allows implementations to impose a ceiling.
/// Linux SEM_NSEMS_MAX is typically 32000; we use 32 as a lightweight default.
pub const SEMAPHORE_TABLE_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// O_CREAT / O_EXCL flags (Linux ABI values)
// ---------------------------------------------------------------------------

/// O_CREAT — create the semaphore if it does not exist.
/// Linux AArch64 value: 0o100 = 64.
pub const O_CREAT: i32 = 64;

/// O_EXCL — fail with EEXIST if the semaphore already exists (used with O_CREAT).
/// Linux AArch64 value: 0o200 = 128.
pub const O_EXCL: i32 = 128;

// ---------------------------------------------------------------------------
// NamedSemaphore
// ---------------------------------------------------------------------------

/// One entry in the global semaphore table.
struct NamedSemaphore {
    /// Fixed-length name buffer (UTF-8, zero-padded).
    name_bytes: [u8; SEMAPHORE_NAME_MAX_LENGTH],
    /// Actual length of the name (no null terminator).
    name_length: usize,
    /// Current semaphore counter value.
    value: u32,
    /// Queue of process IDs blocked in sem_wait (FIFO wake order).
    waiter_queue: VecDeque<Pid>,
    /// Number of open file descriptors referencing this semaphore.
    reference_count: u32,
    /// Set to true by sem_unlink; the slot is freed when reference_count reaches 0.
    is_unlinked: bool,
}

impl NamedSemaphore {
    fn name_matches(&self, name: &[u8]) -> bool {
        name.len() == self.name_length && name == &self.name_bytes[..self.name_length]
    }
}

// ---------------------------------------------------------------------------
// SemaphoreTable
// ---------------------------------------------------------------------------

struct SemaphoreTable {
    entries: [Option<NamedSemaphore>; SEMAPHORE_TABLE_SIZE],
}

impl SemaphoreTable {
    const fn new() -> Self {
        Self {
            entries: [
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
            ],
        }
    }

    /// Find a slot index by name.  Returns None if not found or is unlinked.
    fn find_by_name(&self, name: &[u8]) -> Option<usize> {
        for (index, slot) in self.entries.iter().enumerate() {
            if let Some(semaphore) = slot {
                if !semaphore.is_unlinked && semaphore.name_matches(name) {
                    return Some(index);
                }
            }
        }
        None
    }

    /// Find the first free (None) slot.  Returns None if the table is full.
    fn find_free_slot(&self) -> Option<usize> {
        for (index, slot) in self.entries.iter().enumerate() {
            if slot.is_none() {
                return Some(index);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Global semaphore table
// ---------------------------------------------------------------------------

struct SyncSemaphoreTable(UnsafeCell<SemaphoreTable>);

// SAFETY: Bazzulto OS is single-core with IRQs disabled during all kernel
// operations.  There is never concurrent access from multiple hardware threads.
unsafe impl Sync for SyncSemaphoreTable {}

static SEMAPHORE_TABLE: SyncSemaphoreTable =
    SyncSemaphoreTable(UnsafeCell::new(SemaphoreTable::new()));

/// Run a closure with mutable access to the global semaphore table.
///
/// # Safety
/// Must be called with IRQs disabled (single-core invariant).
unsafe fn with_semaphore_table<F, R>(function: F) -> R
where
    F: FnOnce(&mut SemaphoreTable) -> R,
{
    function(&mut *SEMAPHORE_TABLE.0.get())
}

// ---------------------------------------------------------------------------
// SemaphoreInode — VFS inode wrapping a semaphore table slot
// ---------------------------------------------------------------------------

/// Inode used to represent an open named semaphore as a file descriptor.
///
/// The semaphore operations (wait, post, etc.) are performed through dedicated
/// syscalls.  read_at / write_at return NotSupported.
///
/// The semaphore table index is stored in `table_index` and is also exposed
/// via `InodeStat::nlinks` so that the syscall layer can retrieve it from
/// `Arc<dyn Inode>` without a downcast.
pub struct SemaphoreInode {
    inode_number: u64,
    /// Index into the global SEMAPHORE_TABLE.
    pub table_index: usize,
}

// SAFETY: single-core, IRQs disabled during all accesses.
unsafe impl Send for SemaphoreInode {}
unsafe impl Sync for SemaphoreInode {}

impl SemaphoreInode {
    fn new(table_index: usize) -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            table_index,
        })
    }
}

impl Inode for SemaphoreInode {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: 0,
            // S_IFCHR | 0o666 — character special file.
            mode: 0o020666,
            nlinks: 1,
            uid: 0,
            gid: 0,
        }
    }

    fn ipc_table_index(&self) -> Option<(u8, usize)> {
        Some((1, self.table_index))
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn truncate(&self, _new_size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn Inode>> { None }
    fn readdir(&self, _index: usize) -> Option<DirEntry> { None }

    fn create(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDirectory)
    }

    fn mkdir(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDirectory)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotDirectory)
    }
}

// ---------------------------------------------------------------------------
// Internal helper: retrieve semaphore table index from fd
// ---------------------------------------------------------------------------

/// Retrieve the semaphore table index for the semaphore held by file
/// descriptor `fd` in the current process.
///
/// The index is stored in the `nlinks` field of the inode stat, as set by
/// `SemaphoreInode::stat()`.
///
/// Returns `None` if `fd` is invalid or does not refer to a semaphore inode.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn get_semaphore_table_index(fd: i32) -> Option<usize> {
    if fd < 0 {
        return None;
    }
    let fd_index = fd as usize;
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    })?;
    let guard = fd_table_arc.lock();
    let descriptor = guard.get(fd_index)?;
    if let crate::fs::vfs::FileDescriptor::InoFile { inode, .. } = descriptor {
        // Use the Inode trait's ipc_table_index() method instead of encoding
        // the table index in nlinks. Discriminant 1 = semaphore.
        if let Some((1, candidate_index)) = inode.ipc_table_index() {
            if candidate_index < SEMAPHORE_TABLE_SIZE {
                let slot_occupied = unsafe {
                    with_semaphore_table(|table| table.entries[candidate_index].is_some())
                };
                if slot_occupied {
                    return Some(candidate_index);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Public syscall implementations
// ---------------------------------------------------------------------------

/// sys_sem_open — open or create a named semaphore, return an fd.
///
/// # Safety
/// Must be called with IRQs disabled.  `name_ptr` must be a valid user-space
/// pointer to `name_length` UTF-8 bytes.
pub unsafe fn sys_sem_open(
    name_ptr: u64,
    name_length: usize,
    flags: i32,
    initial_value: u32,
) -> i64 {
    const EINVAL: i64 = -22;
    const ENOENT: i64 = -2;
    const EEXIST: i64 = -17;
    const ENOMEM: i64 = -12;
    const EMFILE: i64 = -24;
    const ESRCH:  i64 = -3;

    if name_ptr == 0 || name_length == 0 || name_length > SEMAPHORE_NAME_MAX_LENGTH {
        return EINVAL;
    }
    if !crate::systemcalls::validate_user_pointer(name_ptr, name_length) {
        return EINVAL;
    }

    // SAFETY: pointer validated above.
    let name_bytes = core::slice::from_raw_parts(name_ptr as *const u8, name_length);

    let table_index_result: Result<usize, i64> = with_semaphore_table(|table| {
        match table.find_by_name(name_bytes) {
            Some(index) => {
                // Semaphore exists.
                if flags & O_CREAT != 0 && flags & O_EXCL != 0 {
                    return Err(EEXIST);
                }
                if let Some(semaphore) = &mut table.entries[index] {
                    semaphore.reference_count += 1;
                }
                Ok(index)
            }
            None => {
                // Semaphore does not exist.
                if flags & O_CREAT == 0 {
                    return Err(ENOENT);
                }
                let free_index = match table.find_free_slot() {
                    Some(i) => i,
                    None => return Err(ENOMEM),
                };
                let mut name_bytes_fixed = [0u8; SEMAPHORE_NAME_MAX_LENGTH];
                name_bytes_fixed[..name_length].copy_from_slice(name_bytes);
                table.entries[free_index] = Some(NamedSemaphore {
                    name_bytes: name_bytes_fixed,
                    name_length,
                    value: initial_value,
                    waiter_queue: VecDeque::new(),
                    reference_count: 1,
                    is_unlinked: false,
                });
                Ok(free_index)
            }
        }
    });

    let table_index = match table_index_result {
        Ok(i) => i,
        Err(errno) => return errno,
    };

    let semaphore_inode = SemaphoreInode::new(table_index);
    let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
        inode: semaphore_inode,
        position: 0,
    };

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    match fd_table_arc {
        Some(arc) => {
            let mut guard = arc.lock();
            let fd = guard.install(descriptor);
            if fd < 0 { EMFILE } else { fd as i64 }
        }
        None => ESRCH,
    }
}

/// sys_sem_close — close a semaphore file descriptor.
///
/// Decrements the semaphore's reference count.  If the semaphore was unlinked
/// and the reference count reaches zero the slot is freed.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_sem_close(fd: i32) -> i64 {
    const EBADF: i64 = -9;

    if fd < 0 {
        return EBADF;
    }
    let fd_index = fd as usize;

    // Retrieve the table index while the fd is still open.
    let table_index = match get_semaphore_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Close the fd (drops the Arc<SemaphoreInode>).
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let closed = match fd_table_arc {
        Some(arc) => {
            let mut guard = arc.lock();
            guard.close(fd_index)
        }
        None => false,
    };
    if !closed {
        return EBADF;
    }

    // Decrement reference count; free the slot if unlinked and count reaches 0.
    with_semaphore_table(|table| {
        if let Some(semaphore) = &mut table.entries[table_index] {
            if semaphore.reference_count > 0 {
                semaphore.reference_count -= 1;
            }
            if semaphore.is_unlinked && semaphore.reference_count == 0 {
                table.entries[table_index] = None;
            }
        }
    });

    0
}

/// sys_sem_wait — blocking decrement of the semaphore counter.
///
/// Blocks the calling process if value == 0.  When unblocked by sem_post the
/// caller owns the token and sem_wait returns 0.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_sem_wait(fd: i32) -> i64 {
    const EBADF: i64 = -9;

    let table_index = match get_semaphore_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    loop {
        let decremented = with_semaphore_table(|table| {
            match &mut table.entries[table_index] {
                Some(semaphore) if semaphore.value > 0 => {
                    semaphore.value -= 1;
                    true
                }
                _ => false,
            }
        });

        if decremented {
            return 0;
        }

        // Value is 0 — enqueue current PID and block until sem_post wakes us.
        crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            // SAFETY: with_semaphore_table accesses a different UnsafeCell and
            // does not re-enter with_scheduler.  IRQs are disabled throughout.
            unsafe {
                with_semaphore_table(|table| {
                    if let Some(semaphore) = &mut table.entries[table_index] {
                        semaphore.waiter_queue.push_back(current_pid);
                    }
                });
            }
            // SAFETY: called with IRQs disabled; sets current process to Blocked
            // state and immediately switches to the next ready process.
            unsafe { scheduler.block_current() };
        });

        // When unblocked by sem_post the token was transferred directly
        // (sem_post dequeued our PID without incrementing value).
        return 0;
    }
}

/// sys_sem_trywait — non-blocking semaphore decrement.
///
/// Returns 0 if decremented, -EAGAIN if value == 0.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_sem_trywait(fd: i32) -> i64 {
    const EBADF:  i64 = -9;
    const EAGAIN: i64 = -11;

    let table_index = match get_semaphore_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    with_semaphore_table(|table| {
        match &mut table.entries[table_index] {
            Some(semaphore) if semaphore.value > 0 => {
                semaphore.value -= 1;
                0
            }
            Some(_) => EAGAIN,
            None => EBADF,
        }
    })
}

/// sys_sem_post — increment the counter or wake one waiter.
///
/// If there are blocked waiters, wake the front of the queue and transfer the
/// token (value is not incremented in that case).  Otherwise increment value.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_sem_post(fd: i32) -> i64 {
    const EBADF: i64 = -9;

    let table_index = match get_semaphore_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    let waiter_pid: Option<Pid> = with_semaphore_table(|table| {
        match &mut table.entries[table_index] {
            Some(semaphore) => {
                if let Some(waiter_pid) = semaphore.waiter_queue.pop_front() {
                    // Transfer token directly; do not increment value.
                    Some(waiter_pid)
                } else {
                    semaphore.value += 1;
                    None
                }
            }
            None => None,
        }
    });

    if let Some(pid) = waiter_pid {
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.unblock(pid);
        });
    }

    0
}

/// sys_sem_unlink — mark a named semaphore for deletion.
///
/// The slot is freed when the reference count reaches zero (i.e. all open
/// sem_close calls have been made).
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_sem_unlink(name_ptr: u64, name_length: usize) -> i64 {
    const EINVAL: i64 = -22;
    const ENOENT: i64 = -2;

    if name_ptr == 0 || name_length == 0 || name_length > SEMAPHORE_NAME_MAX_LENGTH {
        return EINVAL;
    }
    if !crate::systemcalls::validate_user_pointer(name_ptr, name_length) {
        return EINVAL;
    }

    // SAFETY: pointer validated above.
    let name_bytes = core::slice::from_raw_parts(name_ptr as *const u8, name_length);

    with_semaphore_table(|table| {
        let index = match table.find_by_name(name_bytes) {
            Some(i) => i,
            None => return ENOENT,
        };
        if let Some(semaphore) = &mut table.entries[index] {
            semaphore.is_unlinked = true;
            if semaphore.reference_count == 0 {
                table.entries[index] = None;
            }
        }
        0
    })
}

/// sys_sem_getvalue — read the current semaphore counter into a user pointer.
///
/// Writes the current `value` field as a `u32` to the address at `value_ptr`.
///
/// # Safety
/// Must be called with IRQs disabled.  `value_ptr` must be a valid user-space
/// pointer to at least 4 bytes.
pub unsafe fn sys_sem_getvalue(fd: i32, value_ptr: u64) -> i64 {
    const EBADF:  i64 = -9;
    const EINVAL: i64 = -22;
    const EFAULT: i64 = -14;

    let table_index = match get_semaphore_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    if !crate::systemcalls::validate_user_pointer(value_ptr, core::mem::size_of::<u32>()) {
        return EFAULT;
    }

    let current_value: Option<u32> = with_semaphore_table(|table| {
        table.entries[table_index].as_ref().map(|s| s.value)
    });

    match current_value {
        Some(value) => {
            // SAFETY: pointer validated above.
            let destination = &mut *(value_ptr as *mut u32);
            *destination = value;
            0
        }
        None => EINVAL,
    }
}
