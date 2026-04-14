//! Signal handling — rich Signal API with enum variants and associated constants.

use crate::raw;

// ---------------------------------------------------------------------------
// Signal enum
// ---------------------------------------------------------------------------

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Hup  = 1,
    Int  = 2,
    Quit = 3,
    Ill  = 4,
    Abrt = 6,
    Fpe  = 8,
    Kill = 9,
    Usr1 = 10,
    Segv = 11,
    Usr2 = 12,
    Pipe = 13,
    Alrm = 14,
    Term = 15,
    Chld = 17,
    Cont = 18,
    Stop = 19,
}

// rt_sigaction struct layout (152 bytes):
//   offset  0: sa_handler (u64)
//   offset  8: sa_flags   (u64)
//   offset 16: sa_restorer (u64) — must be 0
//   offset 24: sa_mask    ([u64; 16]) = 128 bytes
fn make_sigaction(handler_va: u64, flags: u64) -> [u8; 152] {
    let mut buf = [0u8; 152];
    buf[0..8].copy_from_slice(&handler_va.to_le_bytes());
    buf[8..16].copy_from_slice(&flags.to_le_bytes());
    // sa_restorer at 16 → 0, sa_mask at 24 → all zeros
    buf
}

impl Signal {
    pub const TERM: Signal = Signal::Term;
    pub const KILL: Signal = Signal::Kill;
    pub const INT:  Signal = Signal::Int;
    pub const QUIT: Signal = Signal::Quit;
    pub const HUP:  Signal = Signal::Hup;
    pub const USR1: Signal = Signal::Usr1;
    pub const USR2: Signal = Signal::Usr2;
    pub const PIPE: Signal = Signal::Pipe;
    pub const CHLD: Signal = Signal::Chld;

    /// Send this signal to the given process ID.
    pub fn send(pid: i32, signal: Signal) -> Result<(), i32> {
        let result = raw::raw_kill(pid, signal as i32);
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }

    /// Install a handler for this signal.
    pub fn handle(signal: Signal, handler: unsafe extern "C" fn(i32)) -> Result<(), i32> {
        let act = make_sigaction(handler as u64, 0);
        let result = raw::raw_sigaction(signal as i32, act.as_ptr(), core::ptr::null_mut());
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    /// Ignore this signal (SIG_IGN = 1).
    pub fn ignore(signal: Signal) -> Result<(), i32> {
        let act = make_sigaction(1, 0);
        let result = raw::raw_sigaction(signal as i32, act.as_ptr(), core::ptr::null_mut());
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    /// Restore the default disposition for this signal (SIG_DFL = 0).
    pub fn reset(signal: Signal) -> Result<(), i32> {
        let act = make_sigaction(0, 0);
        let result = raw::raw_sigaction(signal as i32, act.as_ptr(), core::ptr::null_mut());
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }
}

// ---------------------------------------------------------------------------
// Legacy API — kept for existing callers
// ---------------------------------------------------------------------------

pub const SIGHUP:  i32 = 1;
pub const SIGINT:  i32 = 2;
pub const SIGQUIT: i32 = 3;
pub const SIGILL:  i32 = 4;
pub const SIGABRT: i32 = 6;
pub const SIGFPE:  i32 = 8;
pub const SIGKILL: i32 = 9;
pub const SIGSEGV: i32 = 11;
pub const SIGPIPE: i32 = 13;
pub const SIGALRM: i32 = 14;
pub const SIGTERM: i32 = 15;
pub const SIGCHLD: i32 = 17;
pub const SIGCONT: i32 = 18;
pub const SIGSTOP: i32 = 19;

/// Signal action — handler virtual address or special disposition.
#[derive(Clone, Copy, Debug)]
pub enum SigAction {
    Handler(unsafe extern "C" fn(i32)),
    Ignore,
    Default,
}

impl SigAction {
    fn as_va(self) -> u64 {
        match self {
            SigAction::Handler(f) => f as u64,
            SigAction::Ignore  => 1,
            SigAction::Default => 0,
        }
    }
}

/// Install a signal action for `signal_number`.
/// `old_action` receives the previous handler VA if provided.
pub fn sigaction(
    signal_number: i32,
    action: SigAction,
    old_action: Option<&mut u64>,
) -> Result<(), i32> {
    let act = make_sigaction(action.as_va(), 0);
    let mut old_buf = [0u8; 152];
    let old_ptr: *mut u8 = if old_action.is_some() {
        old_buf.as_mut_ptr()
    } else {
        core::ptr::null_mut()
    };
    let result = raw::raw_sigaction(signal_number, act.as_ptr(), old_ptr);
    if result < 0 {
        return Err(result as i32);
    }
    if let Some(slot) = old_action {
        // sa_handler is at offset 0 in the returned struct.
        *slot = u64::from_le_bytes(old_buf[0..8].try_into().unwrap_or([0u8; 8]));
    }
    Ok(())
}
