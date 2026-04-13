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
        let result = raw::raw_sigaction(signal as i32, handler as u64, core::ptr::null_mut());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }

    /// Ignore this signal (SIG_IGN).
    pub fn ignore(signal: Signal) -> Result<(), i32> {
        // SIG_IGN convention: handler VA = 1.
        let result = raw::raw_sigaction(signal as i32, 1u64, core::ptr::null_mut());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }

    /// Restore the default disposition for this signal (SIG_DFL).
    pub fn reset(signal: Signal) -> Result<(), i32> {
        // SIG_DFL convention: handler VA = 0.
        let result = raw::raw_sigaction(signal as i32, 0u64, core::ptr::null_mut());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
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
pub fn sigaction(
    signal_number: i32,
    action: SigAction,
    old_action: Option<&mut u64>,
) -> Result<(), i32> {
    let old_ptr = match old_action {
        Some(r) => r as *mut u64,
        None    => core::ptr::null_mut(),
    };
    let result = raw::raw_sigaction(signal_number, action.as_va(), old_ptr);
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(())
    }
}
