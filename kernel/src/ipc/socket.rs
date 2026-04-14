// ipc/socket.rs — Unix domain sockets (AF_UNIX).
//
// Supports SOCK_STREAM (connection-oriented) and SOCK_DGRAM (connectionless).
// Sockets are identified by a filesystem path (sockaddr_un).  The implementation
// uses a fixed-size global table of UnixSocket structs, following the same
// pattern as the semaphore table in ipc/sem.rs.
//
// Operations:
//   socket    — allocate a new socket, return fd.
//   bind      — attach a socket to a filesystem path.
//   listen    — mark a socket as accepting connections.
//   accept    — accept the next queued connection.
//   connect   — connect to a listening socket.
//   send      — write data into the peer's receive buffer.
//   recv      — read data from this socket's receive buffer.
//   shutdown  — close one or both half-connections.
//   getsockname — return the socket's bound path.
//   getpeername — return the peer's bound path.
//   socketpair  — create two connected sockets without bind/listen.
//
// Access is serialised by IRQ disabling (single-core invariant).
//
// Reference: POSIX.1-2017 §2.10 (Sockets), Linux unix(7).

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::process::Pid;
use crate::fs::inode::{alloc_inode_number, DirEntry, FsError, Inode, InodeStat, InodeType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of simultaneously open Unix domain sockets.
pub const SOCKET_TABLE_SIZE: usize = 64;

/// Maximum number of connections waiting in a listening socket's accept queue.
pub const SOCKET_BACKLOG_MAX: usize = 8;

/// Size of each socket's receive buffer in bytes.
pub const SOCKET_BUFFER_SIZE: usize = 65536;

// ---------------------------------------------------------------------------
// Socket type and state
// ---------------------------------------------------------------------------

/// The communication style of the socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SocketType {
    /// Reliable ordered byte stream (analogous to TCP).
    Stream,
    /// Unreliable unordered datagrams (analogous to UDP).
    Datagram,
}

/// Lifecycle state of a Unix domain socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SocketState {
    /// Newly created; not yet bound to any path.
    Unbound,
    /// Bound to a path; not yet listening.
    Bound,
    /// Listening for incoming connections (SOCK_STREAM only).
    Listening,
    /// Connected to a peer (SOCK_STREAM) or implicitly connected (SOCK_DGRAM).
    Connected,
    /// Socket has been shut down or closed.
    Closed,
}

// ---------------------------------------------------------------------------
// SocketBuffer — ring buffer backed by a Vec
// ---------------------------------------------------------------------------

/// Circular byte buffer used as the socket receive buffer.
///
/// `data` is a Vec allocated to exactly `SOCKET_BUFFER_SIZE` bytes.
/// `read_position` is the index of the first unread byte.
/// `byte_count` is the number of bytes currently buffered.
pub struct SocketBuffer {
    data: Vec<u8>,
    read_position: usize,
    byte_count: usize,
}

impl SocketBuffer {
    fn new(capacity: usize) -> Self {
        let mut backing = Vec::with_capacity(capacity);
        backing.resize(capacity, 0u8);
        Self {
            data: backing,
            read_position: 0,
            byte_count: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.data.len()
    }

    fn available_bytes(&self) -> usize {
        self.byte_count
    }

    fn free_bytes(&self) -> usize {
        self.capacity() - self.byte_count
    }

    /// Write bytes into the ring buffer.  Returns the number of bytes written
    /// (may be less than `source.len()` if the buffer is nearly full).
    fn write(&mut self, source: &[u8]) -> usize {
        let writable = source.len().min(self.free_bytes());
        let capacity = self.capacity();
        let write_start = (self.read_position + self.byte_count) % capacity;
        for (offset, byte) in source[..writable].iter().enumerate() {
            self.data[(write_start + offset) % capacity] = *byte;
        }
        self.byte_count += writable;
        writable
    }

    /// Read bytes from the ring buffer.  Returns the number of bytes read.
    fn read(&mut self, destination: &mut [u8]) -> usize {
        let readable = destination.len().min(self.byte_count);
        let capacity = self.capacity();
        for offset in 0..readable {
            destination[offset] = self.data[(self.read_position + offset) % capacity];
        }
        self.read_position = (self.read_position + readable) % capacity;
        self.byte_count -= readable;
        readable
    }
}

// ---------------------------------------------------------------------------
// UnixSocket
// ---------------------------------------------------------------------------

/// One entry in the global socket table.
pub struct UnixSocket {
    /// Unique inode number assigned at creation.
    pub inode_number: u64,
    /// Whether this is a stream or datagram socket.
    pub socket_type: SocketType,
    /// Current lifecycle state.
    pub state: SocketState,
    /// Filesystem path this socket is bound to (server sockets only).
    pub bound_path: Option<String>,
    /// For connected SOCK_STREAM sockets: index of the peer in SOCKET_TABLE.
    pub peer_index: Option<usize>,
    /// Incoming byte buffer (data sent by the peer, consumed by recv).
    pub receive_buffer: SocketBuffer,
    /// Indices of server-side connected sockets waiting to be accepted.
    pub accept_queue: VecDeque<usize>,
    /// PIDs blocked in recv waiting for data.
    pub receive_waiters: VecDeque<Pid>,
    /// PIDs blocked in accept waiting for a connection.
    pub accept_waiters: VecDeque<Pid>,
    /// True once shutdown(SHUT_RD) or shutdown(SHUT_RDWR) has been called.
    pub shutdown_read: bool,
    /// True once shutdown(SHUT_WR) or shutdown(SHUT_RDWR) has been called.
    pub shutdown_write: bool,
}

impl UnixSocket {
    fn new(socket_type: SocketType) -> Self {
        Self {
            inode_number: alloc_inode_number(),
            socket_type,
            state: SocketState::Unbound,
            bound_path: None,
            peer_index: None,
            receive_buffer: SocketBuffer::new(SOCKET_BUFFER_SIZE),
            accept_queue: VecDeque::new(),
            receive_waiters: VecDeque::new(),
            accept_waiters: VecDeque::new(),
            shutdown_read: false,
            shutdown_write: false,
        }
    }
}

// ---------------------------------------------------------------------------
// SocketTable — fixed-size global array
// ---------------------------------------------------------------------------

struct SocketTable {
    entries: [Option<UnixSocket>; SOCKET_TABLE_SIZE],
}

impl SocketTable {
    const fn new() -> Self {
        // SAFETY: Option<UnixSocket> is not Copy, so we initialise with a
        // manual array expression.  All 64 None values are written at
        // compile time.
        Self {
            entries: [
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
            ],
        }
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
// Global socket table
// ---------------------------------------------------------------------------

struct SyncSocketTable(UnsafeCell<SocketTable>);

// SAFETY: Bazzulto OS is single-core with IRQs disabled during all kernel
// operations.  There is never concurrent access from multiple hardware threads.
unsafe impl Sync for SyncSocketTable {}

static SOCKET_TABLE: SyncSocketTable =
    SyncSocketTable(UnsafeCell::new(SocketTable::new()));

/// Execute a closure with mutable access to the global socket table.
///
/// # Safety
/// Must be called with IRQs disabled (single-core invariant).
unsafe fn with_socket_table<F, R>(function: F) -> R
where
    F: FnOnce(&mut SocketTable) -> R,
{
    function(&mut *SOCKET_TABLE.0.get())
}

// ---------------------------------------------------------------------------
// SocketInode — VFS inode wrapping a socket table slot
// ---------------------------------------------------------------------------

/// VFS inode representing an open Unix domain socket.
///
/// `read_at` delegates to the socket receive path.
/// `write_at` delegates to the socket send path.
/// The socket table index is stored in `InodeStat::nlinks` so that the
/// syscall layer can retrieve it without a downcast.
pub struct SocketInode {
    inode_number: u64,
    /// Index of the corresponding entry in SOCKET_TABLE.
    pub table_index: usize,
}

// SAFETY: single-core, IRQs disabled during all accesses.
unsafe impl Send for SocketInode {}
unsafe impl Sync for SocketInode {}

impl SocketInode {
    pub fn new(table_index: usize) -> Arc<Self> {
        Arc::new(Self {
            inode_number: alloc_inode_number(),
            table_index,
        })
    }
}

impl Inode for SocketInode {
    fn inode_type(&self) -> InodeType {
        // Sockets are exposed as character devices in this kernel's inode model.
        // InodeType::Socket is not defined; CharDevice is the closest analogue
        // for a non-file, non-directory object.
        InodeType::CharDevice
    }

    fn stat(&self) -> InodeStat {
        InodeStat {
            inode_number: self.inode_number,
            size: 0,
            // S_IFSOCK | 0o666 would be 0o140666 in POSIX mode bits.
            // We encode it identically to CharDevice (0o020666) for simplicity;
            // userspace in this kernel does not distinguish socket mode bits.
            mode: 0o020666,
            // Repurpose nlinks to encode the socket table index with discriminant 2.
            nlinks: (2u64 << 32) | self.table_index as u64,
        }
    }

    fn read_at(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        // SAFETY: called from syscall context with IRQs disabled.
        unsafe { socket_receive_bytes(self.table_index, buf) }
    }

    fn write_at(&self, _offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        // SAFETY: called from syscall context with IRQs disabled.
        unsafe { socket_send_bytes(self.table_index, buf) }
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
// Internal helpers
// ---------------------------------------------------------------------------

/// Retrieve the socket table index stored in the inode of file descriptor `fd`.
///
/// Returns `None` if `fd` is invalid or does not refer to a SocketInode.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn get_socket_table_index(fd: i32) -> Option<usize> {
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
        if inode.inode_type() == InodeType::CharDevice {
            let stat = inode.stat();
            // Decode: upper 32 bits = discriminant (2 = socket), lower 32 = index.
            if (stat.nlinks >> 32) != 2 { return None; }
            let candidate_index = (stat.nlinks & 0xFFFF_FFFF) as usize;
            if candidate_index < SOCKET_TABLE_SIZE {
                // Validate the slot is actually occupied (socket exists).
                let slot_occupied = unsafe {
                    with_socket_table(|table| table.entries[candidate_index].is_some())
                };
                if slot_occupied {
                    return Some(candidate_index);
                }
            }
        }
    }
    None
}

/// Low-level receive: copy data from socket's receive buffer into `buf`.
///
/// Blocks if the buffer is empty and the socket is not shut down.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn socket_receive_bytes(table_index: usize, buf: &mut [u8]) -> Result<usize, FsError> {
    loop {
        let result = with_socket_table(|table| {
            let socket = table.entries[table_index].as_mut()?;
            if socket.receive_buffer.available_bytes() > 0 {
                let bytes_read = socket.receive_buffer.read(buf);
                Some(Ok(bytes_read))
            } else if socket.shutdown_read {
                // EOF — the read side has been shut down.
                Some(Ok(0))
            } else {
                None // buffer empty, not shut down — must block
            }
        });

        match result {
            Some(outcome) => return outcome,
            None => {
                // Buffer is empty.  Block until data arrives or shutdown.
                crate::scheduler::with_scheduler(|scheduler| {
                    let current_pid = scheduler.current_pid();
                    unsafe {
                        with_socket_table(|table| {
                            if let Some(socket) = &mut table.entries[table_index] {
                                socket.receive_waiters.push_back(current_pid);
                            }
                        });
                    }
                    // SAFETY: IRQs disabled; switches to next ready process.
                    unsafe { scheduler.block_current() };
                });
                // Re-check after being unblocked.
            }
        }
    }
}

/// Low-level send: write `buf` into the peer socket's receive buffer.
///
/// # Safety
/// Must be called with IRQs disabled.
unsafe fn socket_send_bytes(table_index: usize, buf: &[u8]) -> Result<usize, FsError> {
    // Determine peer index.
    let peer_index = with_socket_table(|table| {
        let socket = table.entries[table_index].as_ref()?;
        if socket.state != SocketState::Connected {
            return None;
        }
        socket.peer_index
    });

    let peer_index = match peer_index {
        Some(index) => index,
        None => return Err(FsError::BrokenPipe),
    };

    let (bytes_written, waiter_to_wake) = with_socket_table(|table| {
        let peer = match table.entries[peer_index].as_mut() {
            Some(socket) => socket,
            None => return (0, None),
        };
        let written = peer.receive_buffer.write(buf);
        let waiter = peer.receive_waiters.pop_front();
        (written, waiter)
    });

    // Wake any process blocked in recv on the peer socket.
    if let Some(waiter_pid) = waiter_to_wake {
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.unblock(waiter_pid);
        });
    }

    if bytes_written == 0 && !buf.is_empty() {
        Err(FsError::WouldBlock)
    } else {
        Ok(bytes_written)
    }
}

// ---------------------------------------------------------------------------
// sockaddr_un layout
// ---------------------------------------------------------------------------

/// Userspace socket address for AF_UNIX.
///
/// Reference: Linux `<sys/un.h>`, POSIX.1-2017 §2.10.12.
#[repr(C)]
struct SockaddrUn {
    /// Address family — must be AF_UNIX (1).
    sun_family: u16,
    /// NUL-terminated filesystem path.
    sun_path: [u8; 108],
}

const SOCKADDR_UN_SIZE: usize = core::mem::size_of::<SockaddrUn>();

/// Extract the NUL-terminated path string from a `SockaddrUn`.
fn sockaddr_un_path(addr: &SockaddrUn) -> &[u8] {
    let nul_pos = addr.sun_path.iter().position(|&b| b == 0)
        .unwrap_or(addr.sun_path.len());
    &addr.sun_path[..nul_pos]
}

// ---------------------------------------------------------------------------
// Public syscall implementations
// ---------------------------------------------------------------------------

/// Error codes used across socket syscalls (negated POSIX errno).
const EAFNOSUPPORT: i64 = -97;
const EPROTONOSUPPORT: i64 = -93;
const ESOCKTNOSUPPORT: i64 = -94;
const ENOTSOCK: i64 = -88;
const EADDRINUSE: i64 = -98;
const EADDRNOTAVAIL: i64 = -99;
const ECONNREFUSED: i64 = -111;
const ENOTCONN: i64 = -107;
const EINVAL: i64 = -22;
const EBADF: i64 = -9;
const ENOMEM: i64 = -12;
const EMFILE: i64 = -24;
const ESRCH: i64 = -3;
const EFAULT: i64 = -14;

/// sys_socket — create a Unix domain socket, return an fd.
///
/// Only `AF_UNIX` (domain=1) is supported.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_socket(domain: i32, socket_type_flags: i32, _protocol: i32) -> i64 {
    // AF_UNIX = 1.
    if domain != 1 {
        return EAFNOSUPPORT;
    }

    // Strip SOCK_NONBLOCK (0x800) and SOCK_CLOEXEC (0x80000) flag bits.
    let base_type = socket_type_flags & !(0x800 | 0x80000);
    let socket_type = match base_type {
        1 => SocketType::Stream,
        2 => SocketType::Datagram,
        _ => return ESOCKTNOSUPPORT,
    };

    let socket = UnixSocket::new(socket_type);

    // Allocate a slot in the global socket table.
    let table_index = match with_socket_table(|table| {
        let index = table.find_free_slot()?;
        table.entries[index] = Some(socket);
        Some(index)
    }) {
        Some(index) => index,
        None => return ENOMEM,
    };

    let socket_inode = SocketInode::new(table_index);
    let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
        inode: socket_inode,
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

/// sys_bind — bind a socket to a filesystem path.
///
/// Creates a SocketInode in the VFS at `path` so that `connect()` can find it.
///
/// # Safety
/// Must be called with IRQs disabled.  `addr_ptr` must point to a valid
/// `SockaddrUn` in user address space.
pub unsafe fn sys_bind(sockfd: i32, addr_ptr: u64, addr_len: usize) -> i64 {
    if addr_len < 3 || addr_len > SOCKADDR_UN_SIZE {
        return EINVAL;
    }
    if !crate::systemcalls::validate_user_pointer(addr_ptr, addr_len) {
        return EFAULT;
    }

    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Read sockaddr_un from user space.
    let addr_bytes = core::slice::from_raw_parts(addr_ptr as *const u8, addr_len);
    if addr_bytes[0] != 1 || addr_bytes[1] != 0 {
        // sun_family must be AF_UNIX = 1 (little-endian u16).
        return EINVAL;
    }

    // Extract the path bytes (sun_path starts at offset 2).
    let path_bytes = &addr_bytes[2..];
    let nul_pos = path_bytes.iter().position(|&b| b == 0).unwrap_or(path_bytes.len());
    let path_slice = &path_bytes[..nul_pos];
    let path_str = match core::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let path_owned: String = {
        let mut s = String::new();
        s.push_str(path_str);
        s
    };

    // Resolve the parent directory and link the socket inode.
    let (parent_inode, file_name) = match crate::fs::vfs_resolve_parent(path_str) {
        Ok(pair) => pair,
        Err(error) => return error.to_errno(),
    };

    // Build the inode that will sit in the VFS.
    let vfs_inode = SocketInode::new(table_index);

    match parent_inode.link_child(&file_name, vfs_inode) {
        Ok(()) => {}
        Err(FsError::AlreadyExists) => return EADDRINUSE,
        Err(error) => return error.to_errno(),
    }

    // Update the socket's state.
    with_socket_table(|table| {
        if let Some(socket) = &mut table.entries[table_index] {
            socket.state = SocketState::Bound;
            socket.bound_path = Some(path_owned);
        }
    });

    0
}

/// sys_listen — mark the socket as accepting connections.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_listen(sockfd: i32, _backlog: i32) -> i64 {
    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    let result = with_socket_table(|table| {
        let socket = match table.entries[table_index].as_mut() {
            Some(s) => s,
            None => return EINVAL,
        };
        if socket.state != SocketState::Bound {
            return EINVAL;
        }
        socket.state = SocketState::Listening;
        0
    });

    result
}

/// sys_accept — accept the next incoming connection.
///
/// Blocks if the accept queue is empty.
/// Returns a new fd for the server-side end of the connection.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_accept(
    sockfd: i32,
    addr_ptr: u64,
    addr_len_ptr: u64,
) -> i64 {
    let listening_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Validate state.
    let is_listening = with_socket_table(|table| {
        table.entries[listening_index].as_ref()
            .map(|s| s.state == SocketState::Listening)
            .unwrap_or(false)
    });
    if !is_listening {
        return EINVAL;
    }

    // Block until a connection arrives.
    loop {
        let connected_socket_index = with_socket_table(|table| {
            table.entries[listening_index].as_mut()?.accept_queue.pop_front()
        });

        if let Some(server_side_index) = connected_socket_index {
            // Optionally write peer address to user space.
            if addr_ptr != 0 && addr_len_ptr != 0 {
                let peer_path: Option<String> = with_socket_table(|table| {
                    let server_socket = table.entries[server_side_index].as_ref()?;
                    let peer_index = server_socket.peer_index?;
                    let peer_socket = table.entries[peer_index].as_ref()?;
                    peer_socket.bound_path.clone()
                });

                if let Some(path) = peer_path {
                    if crate::systemcalls::validate_user_pointer(addr_ptr, SOCKADDR_UN_SIZE)
                        && crate::systemcalls::validate_user_pointer(addr_len_ptr, 4)
                    {
                        let addr_out = addr_ptr as *mut SockaddrUn;
                        // SAFETY: validated above.
                        let addr_ref = &mut *addr_out;
                        addr_ref.sun_family = 1; // AF_UNIX
                        addr_ref.sun_path = [0u8; 108];
                        let path_bytes = path.as_bytes();
                        let copy_len = path_bytes.len().min(107);
                        addr_ref.sun_path[..copy_len].copy_from_slice(&path_bytes[..copy_len]);

                        let actual_len = 2 + copy_len + 1; // family + path + NUL
                        *(addr_len_ptr as *mut u32) = actual_len as u32;
                    }
                }
            }

            // Wrap the server-side socket as a new fd.
            let new_inode = SocketInode::new(server_side_index);
            let descriptor = crate::fs::vfs::FileDescriptor::InoFile {
                inode: new_inode,
                position: 0,
            };

            let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
                scheduler.current_process()
                    .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
            });
            return match fd_table_arc {
                Some(arc) => {
                    let mut guard = arc.lock();
                    let fd = guard.install(descriptor);
                    if fd < 0 { EMFILE } else { fd as i64 }
                }
                None => ESRCH,
            };
        }

        // No connection in queue — block.
        crate::scheduler::with_scheduler(|scheduler| {
            let current_pid = scheduler.current_pid();
            unsafe {
                with_socket_table(|table| {
                    if let Some(socket) = &mut table.entries[listening_index] {
                        socket.accept_waiters.push_back(current_pid);
                    }
                });
            }
            // SAFETY: IRQs disabled; switches to next ready process.
            unsafe { scheduler.block_current() };
        });
    }
}

/// sys_connect — connect to a listening Unix domain socket.
///
/// # Safety
/// Must be called with IRQs disabled.  `addr_ptr` must be a valid
/// `SockaddrUn` in user address space.
pub unsafe fn sys_connect(sockfd: i32, addr_ptr: u64, addr_len: usize) -> i64 {
    if addr_len < 3 || addr_len > SOCKADDR_UN_SIZE {
        return EINVAL;
    }
    if !crate::systemcalls::validate_user_pointer(addr_ptr, addr_len) {
        return EFAULT;
    }

    let client_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    // Read the target path from user space.
    let addr_bytes = core::slice::from_raw_parts(addr_ptr as *const u8, addr_len);
    let path_bytes = &addr_bytes[2..]; // skip sun_family
    let nul_pos = path_bytes.iter().position(|&b| b == 0).unwrap_or(path_bytes.len());
    let path_str = match core::str::from_utf8(&path_bytes[..nul_pos]) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    // Resolve path in VFS to find the listening socket's inode.
    // SAFETY: called from syscall context with IRQs disabled.
    let server_inode = match unsafe { crate::fs::vfs_resolve(path_str, None) } {
        Ok(inode) => inode,
        Err(_) => return ECONNREFUSED,
    };

    if server_inode.inode_type() != InodeType::CharDevice {
        return ECONNREFUSED;
    }

    let server_stat = server_inode.stat();
    // Decode discriminant (2 = socket) + index from nlinks.
    if (server_stat.nlinks >> 32) != 2 {
        return ECONNREFUSED;
    }
    let listening_index = (server_stat.nlinks & 0xFFFF_FFFF) as usize;

    if listening_index >= SOCKET_TABLE_SIZE {
        return ECONNREFUSED;
    }

    // Validate that the target is actually listening.
    let is_listening = with_socket_table(|table| {
        table.entries[listening_index].as_ref()
            .map(|s| s.state == SocketState::Listening)
            .unwrap_or(false)
    });
    if !is_listening {
        return ECONNREFUSED;
    }

    // Allocate a new server-side socket for this specific connection.
    let server_side_index = match with_socket_table(|table| {
        let index = table.find_free_slot()?;
        table.entries[index] = Some(UnixSocket::new(SocketType::Stream));
        Some(index)
    }) {
        Some(i) => i,
        None => return ENOMEM,
    };

    // Wire up the two sockets.
    with_socket_table(|table| {
        if let Some(server_side) = &mut table.entries[server_side_index] {
            server_side.state = SocketState::Connected;
            server_side.peer_index = Some(client_index);
        }
        if let Some(client) = &mut table.entries[client_index] {
            client.state = SocketState::Connected;
            client.peer_index = Some(server_side_index);
        }
    });

    // Enqueue the server-side socket in the listening socket's accept queue.
    let accept_waiter: Option<Pid> = with_socket_table(|table| {
        let listening = table.entries[listening_index].as_mut()?;
        if listening.accept_queue.len() >= SOCKET_BACKLOG_MAX {
            return None; // backlog full — still enqueue but truncate
        }
        listening.accept_queue.push_back(server_side_index);
        listening.accept_waiters.pop_front()
    });

    // Wake an accept waiter if any.
    if let Some(waiter_pid) = accept_waiter {
        crate::scheduler::with_scheduler(|scheduler| {
            scheduler.unblock(waiter_pid);
        });
    }

    0
}

/// sys_send — send data over a connected socket.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_send(sockfd: i32, buf_ptr: u64, length: usize, _flags: i32) -> i64 {
    if !crate::systemcalls::validate_user_pointer(buf_ptr, length) {
        return EFAULT;
    }

    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    // SAFETY: pointer validated above.
    let data = core::slice::from_raw_parts(buf_ptr as *const u8, length);

    match socket_send_bytes(table_index, data) {
        Ok(n) => n as i64,
        Err(error) => error.to_errno(),
    }
}

/// sys_recv — receive data from a connected socket.
///
/// Blocks if the receive buffer is empty and the socket is not shut down.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_recv(sockfd: i32, buf_ptr: u64, length: usize, _flags: i32) -> i64 {
    if !crate::systemcalls::validate_user_pointer(buf_ptr, length) {
        return EFAULT;
    }

    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    // SAFETY: pointer validated above.
    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, length);

    match socket_receive_bytes(table_index, buf) {
        Ok(n) => n as i64,
        Err(error) => error.to_errno(),
    }
}

/// sys_shutdown — shut down part or all of a socket connection.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_shutdown(sockfd: i32, how: i32) -> i64 {
    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    let receive_waiters: VecDeque<Pid> = with_socket_table(|table| {
        let socket = match table.entries[table_index].as_mut() {
            Some(s) => s,
            None => return VecDeque::new(),
        };
        // SHUT_RD = 0, SHUT_WR = 1, SHUT_RDWR = 2
        if how == 0 || how == 2 {
            socket.shutdown_read = true;
        }
        if how == 1 || how == 2 {
            socket.shutdown_write = true;
        }
        // Drain receive waiters so they can see EOF.
        core::mem::take(&mut socket.receive_waiters)
    });

    // Wake all recv-blocked processes so they see EOF.
    crate::scheduler::with_scheduler(|scheduler| {
        for waiter_pid in &receive_waiters {
            scheduler.unblock(*waiter_pid);
        }
    });

    0
}

/// sys_getsockname — write the socket's bound address into user space.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_getsockname(sockfd: i32, addr_ptr: u64, addr_len_ptr: u64) -> i64 {
    if !crate::systemcalls::validate_user_pointer(addr_ptr, SOCKADDR_UN_SIZE) {
        return EFAULT;
    }
    if !crate::systemcalls::validate_user_pointer(addr_len_ptr, 4) {
        return EFAULT;
    }

    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    let bound_path: Option<String> = with_socket_table(|table| {
        table.entries[table_index].as_ref()?.bound_path.clone()
    });

    let addr_out = addr_ptr as *mut SockaddrUn;
    // SAFETY: validated above.
    let addr_ref = &mut *addr_out;
    addr_ref.sun_family = 1; // AF_UNIX

    match bound_path {
        Some(path) => {
            addr_ref.sun_path = [0u8; 108];
            let path_bytes = path.as_bytes();
            let copy_len = path_bytes.len().min(107);
            addr_ref.sun_path[..copy_len].copy_from_slice(&path_bytes[..copy_len]);
            let actual_len = 2 + copy_len + 1;
            *(addr_len_ptr as *mut u32) = actual_len as u32;
        }
        None => {
            addr_ref.sun_path = [0u8; 108];
            *(addr_len_ptr as *mut u32) = 2u32; // just sun_family
        }
    }

    0
}

/// sys_getpeername — write the peer socket's bound address into user space.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_getpeername(sockfd: i32, addr_ptr: u64, addr_len_ptr: u64) -> i64 {
    if !crate::systemcalls::validate_user_pointer(addr_ptr, SOCKADDR_UN_SIZE) {
        return EFAULT;
    }
    if !crate::systemcalls::validate_user_pointer(addr_len_ptr, 4) {
        return EFAULT;
    }

    let table_index = match get_socket_table_index(sockfd) {
        Some(i) => i,
        None => return EBADF,
    };

    let peer_path: Option<String> = with_socket_table(|table| {
        let socket = table.entries[table_index].as_ref()?;
        let peer_index = socket.peer_index?;
        let peer_socket = table.entries[peer_index].as_ref()?;
        peer_socket.bound_path.clone()
    });

    let addr_out = addr_ptr as *mut SockaddrUn;
    // SAFETY: validated above.
    let addr_ref = &mut *addr_out;
    addr_ref.sun_family = 1; // AF_UNIX

    match peer_path {
        Some(path) => {
            addr_ref.sun_path = [0u8; 108];
            let path_bytes = path.as_bytes();
            let copy_len = path_bytes.len().min(107);
            addr_ref.sun_path[..copy_len].copy_from_slice(&path_bytes[..copy_len]);
            let actual_len = 2 + copy_len + 1;
            *(addr_len_ptr as *mut u32) = actual_len as u32;
        }
        None => {
            // Not connected.
            return ENOTCONN;
        }
    }

    0
}

/// sys_socketpair — create two connected sockets without bind/listen/accept.
///
/// `sv_ptr` must point to two contiguous `i32` file descriptor slots in user
/// address space.  On success, the two fds are written there.
///
/// # Safety
/// Must be called with IRQs disabled.
pub unsafe fn sys_socketpair(
    domain: i32,
    socket_type_flags: i32,
    _protocol: i32,
    sv_ptr: u64,
) -> i64 {
    if domain != 1 {
        return EAFNOSUPPORT;
    }
    if !crate::systemcalls::validate_user_pointer(sv_ptr, 8) {
        return EFAULT;
    }

    let base_type = socket_type_flags & !(0x800 | 0x80000);
    let socket_type = match base_type {
        1 => SocketType::Stream,
        2 => SocketType::Datagram,
        _ => return ESOCKTNOSUPPORT,
    };

    // Allocate two sockets.
    let (index_a, index_b) = match with_socket_table(|table| {
        let index_a = table.find_free_slot()?;
        table.entries[index_a] = Some(UnixSocket::new(socket_type));
        let index_b = {
            let mut found = None;
            for i in 0..SOCKET_TABLE_SIZE {
                if i != index_a && table.entries[i].is_none() {
                    found = Some(i);
                    break;
                }
            }
            found?
        };
        table.entries[index_b] = Some(UnixSocket::new(socket_type));
        Some((index_a, index_b))
    }) {
        Some(pair) => pair,
        None => return ENOMEM,
    };

    // Wire them up as a connected pair.
    with_socket_table(|table| {
        if let Some(socket_a) = &mut table.entries[index_a] {
            socket_a.state = SocketState::Connected;
            socket_a.peer_index = Some(index_b);
        }
        if let Some(socket_b) = &mut table.entries[index_b] {
            socket_b.state = SocketState::Connected;
            socket_b.peer_index = Some(index_a);
        }
    });

    // Install both as file descriptors.
    let inode_a = SocketInode::new(index_a);
    let inode_b = SocketInode::new(index_b);

    let fd_table_arc = crate::scheduler::with_scheduler(|scheduler| {
        scheduler.current_process()
            .map(|p| alloc::sync::Arc::clone(&p.file_descriptor_table))
    });
    let (fd_a, fd_b) = match fd_table_arc {
        Some(arc) => {
            let mut guard = arc.lock();
            let fd_a = guard.install(
                crate::fs::vfs::FileDescriptor::InoFile { inode: inode_a, position: 0 }
            );
            let fd_b = guard.install(
                crate::fs::vfs::FileDescriptor::InoFile { inode: inode_b, position: 0 }
            );
            (fd_a, fd_b)
        }
        None => return ESRCH,
    };

    if fd_a < 0 || fd_b < 0 {
        return EMFILE;
    }

    // SAFETY: sv_ptr validated above.
    let sv_out = sv_ptr as *mut i32;
    *sv_out = fd_a;
    *sv_out.add(1) = fd_b;

    0
}
