//! Bazzulto.System — Bazzulto OS syscall and runtime layer.
//!
//! This is the only crate that may issue SVC instructions. All other BSL
//! packages and userspace binaries call through this layer.
//!
//! # Initialization
//!
//! `bazzulto_system::init()` must be called from `_start` before any heap
//! allocation. It installs the global allocator (backed by mmap).

#![no_std]
#![feature(alloc_error_handler)]
#![feature(never_type)]

extern crate alloc;

// Internal
mod alloc_impl;
pub mod raw;
pub mod vdso;

// Public high-level API
pub mod capabilities;
pub mod console;
pub mod diagnostics;
pub mod environment;
pub mod info;
pub mod memory;
pub mod process;
pub mod signal;
pub mod thread;
pub mod time;

// ---------------------------------------------------------------------------
// Process arguments — stored at startup, read by Environment::args()
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicUsize, AtomicPtr, Ordering};
use core::ptr::null_mut;

/// Number of arguments passed to this process (argc).
static PROCESS_ARGC: AtomicUsize = AtomicUsize::new(0);
/// Pointer to the argv[] array (array of pointers to NUL-terminated strings).
static PROCESS_ARGV: AtomicPtr<*const u8> = AtomicPtr::new(null_mut());
/// Pointer to the envp[] array (NULL-terminated array of `KEY=VALUE\0` pointers).
///
/// Set from AArch64 SysV ABI register x2 at process entry.
/// Reference: AArch64 SYSV ABI §3.4.1.
static PROCESS_ENVP: AtomicPtr<*const u8> = AtomicPtr::new(null_mut());

/// Initialize the Bazzulto runtime with AArch64 SYSV ABI arguments and
/// environment.
///
/// Must be called from `_start(argc, argv, envp)` before any heap allocation.
/// `argv` is a pointer to an array of `argc` pointers to NUL-terminated UTF-8
/// strings.  `envp` is a NULL-terminated array of `KEY=VALUE` pointers (x2 on
/// AArch64 SysV ABI entry).
pub fn init_with_args_envp(argc: usize, argv: *const *const u8, envp: *const *const u8) {
    PROCESS_ARGC.store(argc, Ordering::Relaxed);
    PROCESS_ARGV.store(argv as *mut *const u8, Ordering::Relaxed);
    PROCESS_ENVP.store(envp as *mut *const u8, Ordering::Relaxed);
}

/// Initialize the Bazzulto runtime with arguments only (no envp).
///
/// Kept for binaries that have not yet been updated to pass envp.
pub fn init_with_args(argc: usize, argv: *const *const u8) {
    init_with_args_envp(argc, argv, core::ptr::null());
}

/// Initialize the Bazzulto runtime without arguments (for bzinit and other
/// processes that do not receive argv from the kernel).
pub fn init() {
    init_with_args_envp(0, core::ptr::null(), core::ptr::null());
}

/// Return the raw envp pointer as stored at process init.
///
/// The returned pointer is a NULL-terminated array of pointers to
/// NUL-terminated `KEY=VALUE` strings.  Returns null if envp was not set.
pub fn envp_raw() -> *const *const u8 {
    PROCESS_ENVP.load(Ordering::Relaxed) as *const *const u8
}

/// Iterate over the process arguments as UTF-8 string slices.
///
/// Returns an iterator over `&'static str` slices pointing directly into the
/// initial stack page written by the kernel.  The lifetime is `'static`
/// because the stack page persists for the lifetime of the process.
pub fn args() -> impl Iterator<Item = &'static str> {
    let argc = PROCESS_ARGC.load(Ordering::Relaxed);
    let argv = PROCESS_ARGV.load(Ordering::Relaxed) as *const *const u8;
    (0..argc).filter_map(move |index| {
        if argv.is_null() {
            return None;
        }
        // Safety: the kernel wrote argc valid pointers into argv[].
        let ptr = unsafe { *argv.add(index) };
        if ptr.is_null() {
            return None;
        }
        // Walk to find the NUL terminator.
        let mut len = 0usize;
        loop {
            // Safety: kernel wrote NUL-terminated strings.
            if unsafe { *ptr.add(len) } == 0 {
                break;
            }
            len += 1;
            if len > 4096 {
                break; // safety cap
            }
        }
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        core::str::from_utf8(bytes).ok()
    })
}
