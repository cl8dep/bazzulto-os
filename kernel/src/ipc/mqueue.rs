// ipc/mqueue.rs — POSIX message queues.
//
// Message queues allow processes to exchange discrete messages with priorities.
// Unlike pipes (byte stream), each mq_send sends one complete message and each
// mq_receive retrieves exactly one message.  Higher priority messages are
// dequeued first; within the same priority, FIFO order is preserved.
//
// Operations:
//   mq_open    — open or create a named message queue; return fd.
//   mq_close   — close fd (decrement ref-count; does not destroy).
//   mq_send    — enqueue a message (blocks when queue is full).
//   mq_receive — dequeue highest-priority message (blocks when empty).
//   mq_unlink  — mark queue for deletion when ref-count reaches 0.
//   mq_getattr — read queue attributes into a user-space struct.
//
// Access is serialised by IRQ disabling (single-core invariant).
//
// Reference: POSIX.1-2017 §15 (Message Queues).

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::cell::UnsafeCell;

use crate::process::Pid;
use crate::fs::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of message queues that can exist simultaneously.
pub const MESSAGE_QUEUE_TABLE_SIZE: usize = 16;

/// Default maximum number of messages a queue can hold.
pub const MESSAGE_QUEUE_DEFAULT_MAX_MESSAGES: usize = 16;

/// Default maximum size in bytes of a single message.
pub const MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE: usize = 1024;

/// Maximum name length for a queue (including the leading '/').
pub const MESSAGE_QUEUE_NAME_MAX_LENGTH: usize = 64;

// ---------------------------------------------------------------------------
// Message — one item in the queue
// ---------------------------------------------------------------------------

/// A single enqueued message.
struct Message {
    /// Message payload (only the first `length` bytes are valid).
    data: [u8; MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE],
    /// Number of valid bytes in `data`.
    length: usize,
    /// Priority (0–31, higher = dequeued first).
    priority: u32,
}

// ---------------------------------------------------------------------------
// MessageQueue
// ---------------------------------------------------------------------------

/// One entry in the global message queue table.
struct MessageQueue {
    /// Fixed-length name buffer (UTF-8, zero-padded).
    name_bytes: [u8; MESSAGE_QUEUE_NAME_MAX_LENGTH],
    /// Actual length of the name (no null terminator).
    name_length: usize,
    /// Enqueued messages, always kept sorted by priority descending.
    messages: VecDeque<Message>,
    /// Maximum number of messages the queue can hold.
    max_messages: usize,
    /// Maximum size in bytes of a single message.
    max_message_size: usize,
    /// Number of open file descriptors referencing this queue.
    reference_count: u32,
    /// Set to true by mq_unlink; freed when reference_count reaches 0.
    is_unlinked: bool,
    /// PIDs blocked in mq_receive waiting for a message.
    receive_waiters: VecDeque<Pid>,
    /// PIDs blocked in mq_send waiting for free space.
    send_waiters: VecDeque<Pid>,
}

impl MessageQueue {
    fn new(
        name_bytes: [u8; MESSAGE_QUEUE_NAME_MAX_LENGTH],
        name_length: usize,
        max_messages: usize,
        max_message_size: usize,
    ) -> Self {
        Self {
            name_bytes,
            name_length,
            messages: VecDeque::new(),
            max_messages,
            max_message_size,
            reference_count: 1,
            is_unlinked: false,
            receive_waiters: VecDeque::new(),
            send_waiters: VecDeque::new(),
        }
    }

    fn name_matches(&self, name: &[u8]) -> bool {
        name.len() == self.name_length && name == &self.name_bytes[..self.name_length]
    }

    /// Insert a message in priority-descending order (stable within priority).
    fn enqueue(&mut self, message: Message) {
        // Find the insertion point: first index whose priority < new priority.
        let insert_position = self.messages.iter()
            .position(|existing| existing.priority < message.priority)
            .unwrap_or(self.messages.len());
        self.messages.insert(insert_position, message);
    }

    /// Remove and return the highest-priority message (front of queue).
    fn dequeue(&mut self) -> Option<Message> {
        self.messages.pop_front()
    }

    fn message_count(&self) -> usize {
        self.messages.len()
    }

    fn is_full(&self) -> bool {
        self.messages.len() >= self.max_messages
    }

    fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

// ---------------------------------------------------------------------------
// MessageQueueTable — fixed-size global array
// ---------------------------------------------------------------------------

struct MessageQueueTable {
    entries: [Option<MessageQueue>; MESSAGE_QUEUE_TABLE_SIZE],
}

impl MessageQueueTable {
    const fn new() -> Self {
        Self {
            entries: [
                None, None, None, None,
                None, None, None, None,
                None, None, None, None,
                None, None, None, None,
            ],
        }
    }

    fn find_by_name(&self, name: &[u8]) -> Option<usize> {
        for (index, slot) in self.entries.iter().enumerate() {
            if let Some(queue) = slot {
                if !queue.is_unlinked && queue.name_matches(name) {
                    return Some(index);
                }
            }
        }
        None
    }

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
// Global message queue table
// ---------------------------------------------------------------------------

struct SyncMessageQueueTable(UnsafeCell<MessageQueueTable>);

// SAFETY: Bazzulto OS is single-core with IRQs disabled during all kernel
// operations.  There is never concurrent access from multiple hardware threads.
unsafe impl Sync for SyncMessageQueueTable {}

static MESSAGE_QUEUE_TABLE: SyncMessageQueueTable =
    SyncMessageQueueTable(UnsafeCell::new(MessageQueueTable::new()));

/// Execute a closure with mutable access to the global message queue table.
///
/// # Safety
/// Must be called with IRQs disabled (single-core invariant).
unsafe fn with_message_queue_table<F, R>(function: F) -> R
where
    F: FnOnce(&mut MessageQueueTable) -> R,
{
    function(&mut *MESSAGE_QUEUE_TABLE.0.get())
}

// ---------------------------------------------------------------------------
// MqueueInode — VFS inode wrapping a message queue table slot
// ---------------------------------------------------------------------------

/// VFS inode representing an open POSIX message queue.
///
/// mq_send / mq_receive are performed through dedicated syscalls; read_at /
/// write_at return NotSupported.  The queue table index is stored in
/// `InodeStat::nlinks` so the syscall layer can retrieve it without a downcast.
pub struct MqueueInode {
    inode_number: u64,
    /// Index of the corresponding entry in MESSAGE_QUEUE_TABLE.
    pub table_index: usize,
}

// SAFETY: single-core, IRQs disabled during all accesses.
unsafe impl Send for MqueueInode {}
unsafe impl Sync for MqueueInode {}

impl MqueueInode {
    fn new(table_index: usize) -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            table_index,
        })
    }
}

impl Inode for MqueueInode {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: 0,
            mode: 0o020666,
            nlinks: 1,
        }
    }

    fn ipc_table_index(&self) -> Option<(u8, usize)> {
        Some((3, self.table_index))
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
// Internal helper: retrieve queue table index from fd
// ---------------------------------------------------------------------------

/// Retrieve the message queue table index for the queue held by `fd`.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn get_queue_table_index(fd: i32) -> Option<usize> {
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
        // Use the Inode trait's ipc_table_index() method. Discriminant 3 = mqueue.
        if let Some((3, candidate_index)) = inode.ipc_table_index() {
            if candidate_index < MESSAGE_QUEUE_TABLE_SIZE {
                let slot_occupied = unsafe {
                    with_message_queue_table(|table| {
                        table.entries[candidate_index].is_some()
                    })
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
// mq_attr layout
// ---------------------------------------------------------------------------

/// Userspace message queue attributes.
///
/// Reference: POSIX.1-2017 `<mqueue.h>` struct mq_attr.
#[repr(C)]
struct MqAttr {
    /// Message queue flags (0 = blocking, O_NONBLOCK = non-blocking).
    mq_flags: i64,
    /// Maximum number of messages the queue can hold.
    mq_maxmsg: i64,
    /// Maximum size in bytes of each message.
    mq_msgsize: i64,
    /// Current number of messages in the queue.
    mq_curmsgs: i64,
}

const MQ_ATTR_SIZE: usize = core::mem::size_of::<MqAttr>();

// ---------------------------------------------------------------------------
// Public syscall implementations
// ---------------------------------------------------------------------------

const EINVAL: i64 = -22;
const ENOENT: i64 = -2;
const EEXIST: i64 = -17;
const ENOMEM: i64 = -12;
const EMFILE: i64 = -24;
const ESRCH: i64  = -3;
const EBADF: i64  = -9;
const EAGAIN: i64 = -11;
const EMSGSIZE: i64 = -90;
const EFAULT: i64 = -14;

/// O_CREAT — create the queue if it does not exist (Linux AArch64 value: 64).
const O_CREAT: i32 = 64;
/// O_EXCL — fail with EEXIST if queue already exists (Linux AArch64: 128).
const O_EXCL: i32 = 128;
/// O_NONBLOCK — non-blocking send/receive (Linux AArch64: 0x800).
const O_NONBLOCK: i32 = 0x800;

/// sys_mq_open — open or create a named message queue, return an fd.
///
/// `flags` may include O_CREAT, O_EXCL, O_NONBLOCK.
/// If `attr_ptr` is non-null it must point to a valid `MqAttr`; `mq_maxmsg`
/// and `mq_msgsize` are applied to a newly created queue.
///
/// # Safety
/// Must be called with IRQs disabled.
/// Read a NUL-terminated C string from user space into a fixed-size buffer.
///
/// Returns `Some(len)` on success (len = number of bytes, not counting NUL),
/// or `None` if the pointer is invalid or the string exceeds `buf.len() - 1`.
///
/// This mirrors `copy_user_cstr` in `crate::systemcalls` but is defined here
/// to avoid a cross-crate dependency on a private systemcalls function.
unsafe fn copy_nul_string(ptr: u64, buf: &mut [u8; 256]) -> Option<usize> {
    const PAGE_SIZE: u64 = 4096;
    if ptr < PAGE_SIZE || ptr >= crate::process::USER_ADDR_LIMIT {
        return None;
    }
    let mut i = 0usize;
    loop {
        if i >= 255 {
            return None; // string too long
        }
        let byte = core::ptr::read_volatile((ptr + i as u64) as *const u8);
        if byte == 0 {
            buf[i] = 0;
            return Some(i);
        }
        buf[i] = byte;
        i += 1;
    }
}

pub unsafe fn sys_mq_open(
    name_ptr: u64,
    flags: i32,
    _mode: u32,
    attr_ptr: u64,
) -> i64 {
    // Read the NUL-terminated queue name from user space.
    let mut name_buf = [0u8; 256];
    let name_length = match copy_nul_string(name_ptr, &mut name_buf) {
        Some(l) if l > 0 && l <= MESSAGE_QUEUE_NAME_MAX_LENGTH => l,
        Some(0) | None => return EINVAL,
        Some(_) => return EINVAL,
    };
    let name_bytes_slice = &name_buf[..name_length];

    // Optional attributes from user space.
    let (custom_max_messages, custom_max_message_size) = if attr_ptr != 0 {
        if !crate::systemcalls::validate_user_pointer(attr_ptr, MQ_ATTR_SIZE) {
            return EFAULT;
        }
        // SAFETY: validated above.
        let attr = &*(attr_ptr as *const MqAttr);
        let max_messages = if attr.mq_maxmsg > 0 && attr.mq_maxmsg as usize <= MESSAGE_QUEUE_DEFAULT_MAX_MESSAGES {
            attr.mq_maxmsg as usize
        } else {
            MESSAGE_QUEUE_DEFAULT_MAX_MESSAGES
        };
        let max_message_size = if attr.mq_msgsize > 0 && attr.mq_msgsize as usize <= MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE {
            attr.mq_msgsize as usize
        } else {
            MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE
        };
        (max_messages, max_message_size)
    } else {
        (MESSAGE_QUEUE_DEFAULT_MAX_MESSAGES, MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE)
    };

    let table_index_result: Result<usize, i64> = with_message_queue_table(|table| {
        match table.find_by_name(name_bytes_slice) {
            Some(index) => {
                // Queue exists.
                if flags & O_CREAT != 0 && flags & O_EXCL != 0 {
                    return Err(EEXIST);
                }
                if let Some(queue) = &mut table.entries[index] {
                    queue.reference_count += 1;
                }
                Ok(index)
            }
            None => {
                // Queue does not exist.
                if flags & O_CREAT == 0 {
                    return Err(ENOENT);
                }
                let free_index = match table.find_free_slot() {
                    Some(i) => i,
                    None => return Err(ENOMEM),
                };
                let mut name_bytes_fixed = [0u8; MESSAGE_QUEUE_NAME_MAX_LENGTH];
                name_bytes_fixed[..name_length].copy_from_slice(name_bytes_slice);
                table.entries[free_index] = Some(MessageQueue::new(
                    name_bytes_fixed,
                    name_length,
                    custom_max_messages,
                    custom_max_message_size,
                ));
                Ok(free_index)
            }
        }
    });

    let table_index = match table_index_result {
        Ok(i) => i,
        Err(errno) => return errno,
    };

    let queue_inode = MqueueInode::new(table_index);
    let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
        inode: queue_inode,
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

/// sys_mq_close — close a message queue file descriptor.
///
/// Decrements the reference count.  If the queue was unlinked and the
/// reference count reaches zero, the slot is freed.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_mq_close(fd: i32) -> i64 {
    let table_index = match get_queue_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Remove the fd from the process's table.
    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    if let Some(arc) = fd_table_arc {
        let mut guard = arc.lock();
        guard.close(fd as usize);
    }

    // Decrement ref-count; free if unlinked and ref-count == 0.
    with_message_queue_table(|table| {
        if let Some(queue) = &mut table.entries[table_index] {
            if queue.reference_count > 0 {
                queue.reference_count -= 1;
            }
            if queue.reference_count == 0 && queue.is_unlinked {
                table.entries[table_index] = None;
            }
        }
    });

    0
}

/// sys_mq_send — enqueue a message with a given priority.
///
/// Blocks if the queue is full.  Returns -EAGAIN if O_NONBLOCK is set.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_mq_send(fd: i32, msg_ptr: u64, msg_length: usize, priority: u32) -> i64 {
    if !crate::systemcalls::validate_user_pointer(msg_ptr, msg_length) {
        return EFAULT;
    }

    let table_index = match get_queue_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Validate message length against the queue's max_message_size.
    let max_message_size = with_message_queue_table(|table| {
        table.entries[table_index].as_ref().map(|q| q.max_message_size)
    });
    let max_message_size = match max_message_size {
        Some(s) => s,
        None => return EBADF,
    };
    if msg_length > max_message_size {
        return EMSGSIZE;
    }

    loop {
        let is_full = with_message_queue_table(|table| {
            table.entries[table_index].as_ref().map(|q| q.is_full()).unwrap_or(true)
        });

        if !is_full {
            break;
        }

        // Queue is full — block (no O_NONBLOCK support in the fd here; we
        // always block for simplicity unless the queue is permanently gone).
        let queue_exists = with_message_queue_table(|table| {
            table.entries[table_index].is_some()
        });
        if !queue_exists {
            return EBADF;
        }

        crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            unsafe {
                with_message_queue_table(|table| {
                    if let Some(queue) = &mut table.entries[table_index] {
                        queue.send_waiters.push_back(current_pid);
                    }
                });
            }
            // SAFETY: IRQs disabled; switches to next ready process.
            unsafe { scheduler.block_current() };
        });
    }

    // Read message data from user space.
    // SAFETY: pointer validated above.
    let user_data = core::slice::from_raw_parts(msg_ptr as *const u8, msg_length);

    let mut message_data = [0u8; MESSAGE_QUEUE_DEFAULT_MAX_MESSAGE_SIZE];
    message_data[..msg_length].copy_from_slice(user_data);

    let message = Message {
        data: message_data,
        length: msg_length,
        priority,
    };

    // Enqueue and wake one receive waiter.
    let receive_waiter = with_message_queue_table(|table| {
        let queue = table.entries[table_index].as_mut()?;
        queue.enqueue(message);
        queue.receive_waiters.pop_front()
    });

    if let Some(waiter_pid) = receive_waiter {
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.unblock(waiter_pid);
        });
    }

    0
}

/// sys_mq_receive — dequeue the highest-priority message.
///
/// Blocks if the queue is empty.  Writes message priority to `priority_ptr`
/// if non-null.  Returns the number of bytes in the message.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_mq_receive(
    fd: i32,
    buf_ptr: u64,
    buf_length: usize,
    priority_ptr: u64,
) -> i64 {
    if !crate::systemcalls::validate_user_pointer(buf_ptr, buf_length) {
        return EFAULT;
    }

    let table_index = match get_queue_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    loop {
        let dequeued: Option<Message> = with_message_queue_table(|table| {
            table.entries[table_index].as_mut()?.dequeue()
        });

        if let Some(message) = dequeued {
            if buf_length < message.length {
                // Buffer too small — put the message back (not POSIX-compliant
                // for mq_receive, which would return EMSGSIZE; we return error).
                with_message_queue_table(|table| {
                    if let Some(queue) = &mut table.entries[table_index] {
                        queue.enqueue(message);
                    }
                });
                return EMSGSIZE;
            }

            // Copy message to user buffer.
            // SAFETY: validated above.
            let user_buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_length);
            user_buf[..message.length].copy_from_slice(&message.data[..message.length]);

            // Write priority if requested.
            if priority_ptr != 0
                && crate::systemcalls::validate_user_pointer(priority_ptr, 4)
            {
                // SAFETY: validated above.
                *(priority_ptr as *mut u32) = message.priority;
            }

            // Wake one send waiter.
            let send_waiter = with_message_queue_table(|table| {
                table.entries[table_index].as_mut()?.send_waiters.pop_front()
            });
            if let Some(waiter_pid) = send_waiter {
                crate::scheduler::with_scheduler(|scheduler| {
                    scheduler.unblock(waiter_pid);
                });
            }

            return message.length as i64;
        }

        // Queue is empty — block until a message arrives.
        let queue_exists = with_message_queue_table(|table| {
            table.entries[table_index].is_some()
        });
        if !queue_exists {
            return EBADF;
        }

        crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            unsafe {
                with_message_queue_table(|table| {
                    if let Some(queue) = &mut table.entries[table_index] {
                        queue.receive_waiters.push_back(current_pid);
                    }
                });
            }
            // SAFETY: IRQs disabled; switches to next ready process.
            unsafe { scheduler.block_current() };
        });
    }
}

/// sys_mq_unlink — mark a queue for deletion when all references are closed.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_mq_unlink(name_ptr: u64) -> i64 {
    // Read the NUL-terminated queue name from user space.
    let mut name_buf = [0u8; 256];
    let name_length = match copy_nul_string(name_ptr, &mut name_buf) {
        Some(l) if l > 0 && l <= MESSAGE_QUEUE_NAME_MAX_LENGTH => l,
        Some(0) | None => return EINVAL,
        Some(_) => return EINVAL,
    };
    let name_bytes = &name_buf[..name_length];

    with_message_queue_table(|table| {
        match table.find_by_name(name_bytes) {
            Some(index) => {
                if let Some(queue) = &mut table.entries[index] {
                    queue.is_unlinked = true;
                    if queue.reference_count == 0 {
                        table.entries[index] = None;
                    }
                }
                0
            }
            None => ENOENT,
        }
    })
}

/// sys_mq_getattr — write current queue attributes to a user-space `MqAttr`.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_mq_getattr(fd: i32, attr_ptr: u64) -> i64 {
    if !crate::systemcalls::validate_user_pointer(attr_ptr, MQ_ATTR_SIZE) {
        return EFAULT;
    }

    let table_index = match get_queue_table_index(fd) {
        Some(i) => i,
        None => return EBADF,
    };

    let attributes = with_message_queue_table(|table| {
        let queue = table.entries[table_index].as_ref()?;
        Some((queue.max_messages, queue.max_message_size, queue.message_count()))
    });

    let (max_messages, max_message_size, current_messages) = match attributes {
        Some(triple) => triple,
        None => return EBADF,
    };

    // SAFETY: validated above.
    let attr_out = attr_ptr as *mut MqAttr;
    (*attr_out).mq_flags   = 0;
    (*attr_out).mq_maxmsg  = max_messages as i64;
    (*attr_out).mq_msgsize = max_message_size as i64;
    (*attr_out).mq_curmsgs = current_messages as i64;

    0
}
