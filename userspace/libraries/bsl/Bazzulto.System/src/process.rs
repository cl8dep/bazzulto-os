//! Process management — spawn, fork, exec, wait, exit, and high-level builders.

use alloc::string::String;
use alloc::vec::Vec;
use crate::raw;

// ---------------------------------------------------------------------------
// Pipe
// ---------------------------------------------------------------------------

/// A pipe — pair of (read_fd, write_fd).
pub struct Pipe {
    pub read_fd:  i32,
    pub write_fd: i32,
}

impl Pipe {
    /// Create a new pipe. Returns `Ok(Pipe)` or `Err(errno)`.
    pub fn new() -> Result<Pipe, i32> {
        let mut fd_pair = [0i32; 2];
        let result = raw::raw_pipe(fd_pair.as_mut_ptr());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(Pipe { read_fd: fd_pair[0], write_fd: fd_pair[1] })
        }
    }

    /// Consume the pipe without closing the fds.
    pub fn into_raw(self) -> (i32, i32) {
        let pair = (self.read_fd, self.write_fd);
        core::mem::forget(self);
        pair
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        raw::raw_close(self.read_fd);
        raw::raw_close(self.write_fd);
    }
}

// ---------------------------------------------------------------------------
// ProcessBuilder
// ---------------------------------------------------------------------------

/// Builder for spawning a child process.
pub struct ProcessBuilder {
    path:     String,
    stdin_fd: Option<i32>,
    stdout_fd: Option<i32>,
    stderr_fd: Option<i32>,
    envs: Vec<(String, String)>,
}

impl ProcessBuilder {
    fn new(path: &str) -> ProcessBuilder {
        ProcessBuilder {
            path:      String::from(path),
            stdin_fd:  None,
            stdout_fd: None,
            stderr_fd: None,
            envs:      Vec::new(),
        }
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.envs.push((String::from(key), String::from(value)));
        self
    }

    /// Use `pipe.read_fd` as the child's stdin.
    pub fn stdin(mut self, pipe: Pipe) -> Self {
        let (read_fd, _write_fd) = pipe.into_raw();
        self.stdin_fd = Some(read_fd);
        self
    }

    /// Use `pipe.write_fd` as the child's stdout.
    pub fn stdout(mut self, pipe: Pipe) -> Self {
        let (_read_fd, write_fd) = pipe.into_raw();
        self.stdout_fd = Some(write_fd);
        self
    }

    /// Use `pipe.write_fd` as the child's stderr.
    pub fn stderr(mut self, pipe: Pipe) -> Self {
        let (_read_fd, write_fd) = pipe.into_raw();
        self.stderr_fd = Some(write_fd);
        self
    }

    /// Fork + dup2 + exec. Returns `SpawnedProcess` on success.
    pub fn run(self) -> Result<SpawnedProcess, i32> {
        let pid = match fork() {
            Ok(p) => p,
            Err(e) => return Err(e),
        };

        if pid == 0 {
            // Child: set up fds, then exec.
            if let Some(fd) = self.stdin_fd {
                raw::raw_dup2(fd, 0);
                raw::raw_close(fd);
            }
            if let Some(fd) = self.stdout_fd {
                raw::raw_dup2(fd, 1);
                raw::raw_close(fd);
            }
            if let Some(fd) = self.stderr_fd {
                raw::raw_dup2(fd, 2);
                raw::raw_close(fd);
            }
            let _ = exec(&self.path);
            // exec failed — exit child.
            raw::raw_exit(1);
        }

        // Parent: close child-end fds.
        if let Some(fd) = self.stdin_fd {
            raw::raw_close(fd);
        }
        if let Some(fd) = self.stdout_fd {
            raw::raw_close(fd);
        }
        if let Some(fd) = self.stderr_fd {
            raw::raw_close(fd);
        }

        Ok(SpawnedProcess { pid })
    }
}

// ---------------------------------------------------------------------------
// SpawnedProcess
// ---------------------------------------------------------------------------

/// A spawned child process.
pub struct SpawnedProcess {
    pub pid: i32,
}

impl SpawnedProcess {
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Wait for this process to exit. Returns the exit status.
    pub fn wait(&self) -> Result<i32, i32> {
        let mut status: i32 = 0;
        let result = raw::raw_wait(self.pid, &mut status as *mut i32, 0);
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(status)
        }
    }

    /// Send a signal to this process.
    pub fn kill(&self, sig: i32) -> Result<(), i32> {
        let result = raw::raw_kill(self.pid, sig);
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// CurrentProcess
// ---------------------------------------------------------------------------

/// Handle to the current process.
pub struct CurrentProcess;

impl CurrentProcess {
    pub fn pid(&self) -> i32 {
        raw::raw_getpid() as i32
    }

    pub fn ppid(&self) -> i32 {
        raw::raw_getppid() as i32
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn name(&self) -> Option<&'static str> {
        None
    }

    /// Deferred — requires kernel sysinfo syscall (see docs/tech-debt/bzinit-v1.md).
    pub fn memory_usage(&self) -> Option<u64> {
        None
    }
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

/// Process management facade.
pub struct Process;

impl Process {
    /// Create a `ProcessBuilder` for the given binary path.
    pub fn spawn(path: &str, _args: &[&str]) -> ProcessBuilder {
        ProcessBuilder::new(path)
    }

    /// Handle to the current process.
    pub fn current() -> CurrentProcess {
        CurrentProcess
    }

    /// Terminate the current process. Never returns.
    pub fn exit(code: i32) -> ! {
        raw::raw_exit(code)
    }

    /// Deferred — requires kernel process lookup syscall (ENOSYS).
    pub fn find(_pid: i32) -> Result<SpawnedProcess, i32> {
        Err(-38) // ENOSYS
    }

    /// Deferred — requires kernel process list syscall (ENOSYS).
    pub fn list() -> Result<Vec<i32>, i32> {
        Err(-38) // ENOSYS
    }
}

// ---------------------------------------------------------------------------
// Low-level helpers (kept for internal use and legacy callers)
// ---------------------------------------------------------------------------

/// Terminate the current process with the given exit code. Never returns.
#[inline]
pub fn exit(code: i32) -> ! {
    raw::raw_exit(code)
}

/// Return the current process ID.
#[inline]
pub fn getpid() -> i32 {
    raw::raw_getpid() as i32
}

/// Return the parent process ID.
#[inline]
pub fn getppid() -> i32 {
    raw::raw_getppid() as i32
}

/// Fork the current process. Returns `Ok(0)` in child, `Ok(child_pid)` in parent.
#[inline]
pub fn fork() -> Result<i32, i32> {
    let result = raw::raw_fork();
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(result as i32)
    }
}

/// Replace the current process image with a ramfs binary.
/// Passes no arguments and no environment.
#[inline]
pub fn exec(path: &str) -> Result<!, i32> {
    let mut buf = [0u8; 512];
    let len = path.len().min(511);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    // argv: [path_ptr, null]  envp: [null]
    let argv: [*const u8; 2] = [buf.as_ptr(), core::ptr::null()];
    let envp: [*const u8; 1] = [core::ptr::null()];
    let result = raw::raw_exec(buf.as_ptr(), argv.as_ptr(), envp.as_ptr());
    Err(result as i32)
}

/// Spawn a child process from a ramfs path. Returns child PID on success.
#[inline]
pub fn spawn(path: &str) -> Result<i32, i32> {
    let mut buf = [0u8; 512];
    let len = path.len().min(511);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    let result = raw::raw_spawn(buf.as_ptr());
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(result as i32)
    }
}

/// Wait for a child process (blocking).
#[inline]
pub fn wait(pid: i32) -> Result<(i32, i32), i32> {
    let mut status: i32 = 0;
    let result = raw::raw_wait(pid, &mut status as *mut i32, 0);
    if result < 0 {
        Err(result as i32)
    } else {
        Ok((result as i32, status))
    }
}

/// Send a signal to a process.
#[inline]
pub fn kill(pid: i32, signal_number: i32) -> Result<(), i32> {
    let result = raw::raw_kill(pid, signal_number);
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(())
    }
}
