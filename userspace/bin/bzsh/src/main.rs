//! bzsh — Bazzulto Shell
//!
//! A POSIX-flavoured interactive shell for Bazzulto OS, written in Rust
//! and using the Bazzulto Standard Library (vDSO-backed syscalls, GlobalAlloc).
//!
//! Features:
//!   - Tokenizer with single- and double-quote handling
//!   - Pipelines: `cmd1 | cmd2 | cmd3`
//!   - I/O redirects: `< file`, `> file`, `>> file`
//!   - SIGINT forwarded to the foreground process group, not to the shell
//!   - Built-in: `exit [code]`

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use bazzulto_system::raw;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    // Start in /home; fall back to / if the directory doesn't exist yet.
    let home = b"/home";
    if raw::raw_chdir(home.as_ptr(), home.len()) < 0 {
        let root = b"/";
        let _ = raw::raw_chdir(root.as_ptr(), root.len());
    }
    shell_main();
    raw::raw_exit(0)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_str(fd: i32, s: &str) {
    raw::raw_write(fd, s.as_ptr(), s.len());
}

fn write_stdout(s: &str) {
    write_str(1, s);
}

fn write_stderr(s: &str) {
    write_str(2, s);
}

/// Read one full line from stdin. Strips trailing `\n`.
/// Returns an empty String on EOF.
fn read_line() -> String {
    let mut buf = [0u8; 256];
    let n = raw::raw_read(0, buf.as_mut_ptr(), buf.len());
    if n <= 0 {
        return String::new();
    }
    let n = n as usize;
    // Strip trailing newline.
    let end = if n > 0 && buf[n - 1] == b'\n' { n - 1 } else { n };
    core::str::from_utf8(&buf[..end])
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// Split `input` on whitespace, respecting single- and double-quoted spans.
/// Returns `Err(())` on unterminated quote.
fn tokenize(input: &str) -> Result<Vec<String>, ()> {
    #[derive(PartialEq)]
    enum State { Normal, InDouble, InSingle }

    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut state = State::Normal;

    for character in input.chars() {
        match state {
            State::Normal => {
                if character == ' ' || character == '\t' {
                    if in_token {
                        tokens.push(current.clone());
                        current.clear();
                        in_token = false;
                    }
                } else if character == '"' {
                    in_token = true;
                    state = State::InDouble;
                } else if character == '\'' {
                    in_token = true;
                    state = State::InSingle;
                } else {
                    in_token = true;
                    current.push(character);
                }
            }
            State::InDouble => {
                if character == '"' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
            State::InSingle => {
                if character == '\'' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
        }
    }

    if state != State::Normal {
        return Err(());
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Pipeline splitting
// ---------------------------------------------------------------------------

/// Split `line` on unquoted `|` characters.
/// Returns a `Vec` of stage strings (each is a sub-command).
fn split_pipeline(line: &str) -> Vec<String> {
    let mut stages: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_single = false;

    for character in line.chars() {
        match character {
            '"' if !in_single => {
                in_double = !in_double;
                current.push(character);
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(character);
            }
            '|' if !in_double && !in_single => {
                stages.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(character);
            }
        }
    }
    stages.push(current);
    stages
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

/// Expand a bare command name to `/system/bin/<name>` if it has no `/`.
fn resolve_path(cmd: &str) -> String {
    if cmd.contains('/') {
        cmd.to_string()
    } else {
        let mut path = String::from("/system/bin/");
        path.push_str(cmd);
        path
    }
}

// ---------------------------------------------------------------------------
// Built-in commands
// ---------------------------------------------------------------------------

/// Handle built-in commands. Returns `Some(exit_code)` if handled, `None` if
/// the command should be dispatched to an external binary.
fn try_builtin(tokens: &[String]) -> Option<i32> {
    if tokens.is_empty() {
        return Some(0);
    }
    let cmd = tokens[0].as_str();
    let args: alloc::vec::Vec<&str> = tokens[1..].iter().map(|s| s.as_str()).collect();
    match cmd {
        "cd" => {
            let path = args.first().copied().unwrap_or("/home");
            let result = raw::raw_chdir(path.as_ptr(), path.len());
            if result < 0 {
                write_stderr(&alloc::format!("cd: {}: No such file or directory\n", path));
            }
            Some(if result < 0 { 1 } else { 0 })
        }
        "pwd" => {
            let mut buf = [0u8; 512];
            let n = raw::raw_getcwd(buf.as_mut_ptr(), buf.len());
            if n > 1 {
                let len = (n as usize).saturating_sub(1);
                if let Ok(path) = core::str::from_utf8(&buf[..len]) {
                    write_stdout(path);
                    write_stdout("\n");
                }
            }
            Some(0)
        }
        "echo" => {
            for (i, arg) in args.iter().enumerate() {
                if i > 0 { write_stdout(" "); }
                write_stdout(arg);
            }
            write_stdout("\n");
            Some(0)
        }
        "exit" => {
            let code: i32 = args.first()
                .and_then(|s| parse_i32(s))
                .unwrap_or(0);
            raw::raw_exit(code)
        }
        "export" | "setenv" => {
            // TODO: implement when env var syscall is available
            Some(0)
        }
        "jobs" => {
            write_stdout("(no background jobs)\n");
            Some(0)
        }
        "bg" | "fg" => {
            write_stderr(&alloc::format!("{}: not supported\n", cmd));
            Some(1)
        }
        _ => None,
    }
}

fn parse_i32(s: &str) -> Option<i32> {
    let mut result: i32 = 0;
    let mut negative = false;
    let mut chars = s.chars();
    let first = chars.next()?;
    if first == '-' {
        negative = true;
    } else if let Some(d) = first.to_digit(10) {
        result = d as i32;
    } else {
        return None;
    }
    for character in chars {
        let digit = character.to_digit(10)? as i32;
        result = result.checked_mul(10)?.checked_add(digit)?;
    }
    Some(if negative { -result } else { result })
}

// ---------------------------------------------------------------------------
// Stage execution
// ---------------------------------------------------------------------------

/// Open a file for reading. Returns fd or negative errno.
fn open_file(path: &str) -> i64 {
    raw::raw_open(path.as_ptr(), path.len())
}

/// Create or truncate a file for writing. Returns fd or negative errno.
fn create_file(path: &str) -> i64 {
    raw::raw_creat(path.as_ptr(), path.len())
}

/// Execute a single pipeline stage.
///
/// - `in_fd`  — fd to use as stdin  (0 = inherited)
/// - `out_fd` — fd to use as stdout (1 = inherited)
///
/// Returns the child PID on success, -1 on error.
fn execute_stage(stage: &str, in_fd: i32, out_fd: i32) -> i32 {
    let tokens = match tokenize(stage) {
        Ok(t) => t,
        Err(()) => {
            write_stderr("bzsh: unterminated quote\r\n");
            return -1;
        }
    };

    if tokens.is_empty() {
        return -1;
    }

    // Separate redirect tokens from the command.
    let mut command_tokens: Vec<String> = Vec::new();
    let mut redirect_in_fd:  i32 = -1;
    let mut redirect_out_fd: i32 = -1;
    let mut i = 0usize;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "<" => {
                if i + 1 >= tokens.len() {
                    write_stderr("bzsh: expected filename after '<'\r\n");
                    return -1;
                }
                let fd = open_file(&tokens[i + 1]);
                if fd < 0 {
                    write_stderr(&tokens[i + 1]);
                    write_stderr(": no such file\r\n");
                    return -1;
                }
                redirect_in_fd = fd as i32;
                i += 2;
            }
            ">>" => {
                if i + 1 >= tokens.len() {
                    write_stderr("bzsh: expected filename after '>>'\r\n");
                    return -1;
                }
                let path = &tokens[i + 1];
                let fd = raw::raw_creat_append(path.as_ptr(), path.len());
                if fd < 0 {
                    write_stderr(path);
                    write_stderr(": cannot open\r\n");
                    return -1;
                }
                // Seek to end for append.
                raw::raw_seek(fd as i32, 0, 2);
                redirect_out_fd = fd as i32;
                i += 2;
            }
            ">" => {
                if i + 1 >= tokens.len() {
                    write_stderr("bzsh: expected filename after '>'\r\n");
                    return -1;
                }
                let fd = create_file(&tokens[i + 1]);
                if fd < 0 {
                    write_stderr(&tokens[i + 1]);
                    write_stderr(": cannot create\r\n");
                    return -1;
                }
                redirect_out_fd = fd as i32;
                i += 2;
            }
            _ => {
                command_tokens.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    if command_tokens.is_empty() {
        if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
        if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }
        return -1;
    }

    // Determine if the command is a builtin before forking/execing.
    let cmd_name = command_tokens[0].as_str();

    // cd and exit must run in the shell process itself (they mutate shell
    // state).  Redirects on them are silently ignored — `cd /foo > f` is
    // not meaningful.
    if cmd_name == "cd" || cmd_name == "exit" {
        if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
        if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }
        try_builtin(&command_tokens);
        return -1; // no child PID
    }

    // All other builtins (echo, pwd, jobs, bg, fg, export …) can run in a
    // forked child so that I/O redirects work transparently.
    let is_output_builtin = matches!(
        cmd_name,
        "echo" | "pwd" | "jobs" | "bg" | "fg" | "export" | "setenv"
    );
    if is_output_builtin {
        let fork_result = raw::raw_fork();
        if fork_result < 0 {
            write_stderr("bzsh: fork failed\r\n");
            if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
            if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }
            return -1;
        }
        if fork_result == 0 {
            if in_fd != 0           { raw::raw_dup2(in_fd, 0);           raw::raw_close(in_fd); }
            if out_fd != 1          { raw::raw_dup2(out_fd, 1);          raw::raw_close(out_fd); }
            if redirect_in_fd >= 0  { raw::raw_dup2(redirect_in_fd, 0);  raw::raw_close(redirect_in_fd); }
            if redirect_out_fd >= 0 { raw::raw_dup2(redirect_out_fd, 1); raw::raw_close(redirect_out_fd); }
            let code = try_builtin(&command_tokens).unwrap_or(0);
            raw::raw_exit(code);
        }
        if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
        if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }
        return fork_result as i32;
    }

    let path = resolve_path(&command_tokens[0]);

    // Fork.
    let fork_result = raw::raw_fork();
    if fork_result < 0 {
        write_stderr("bzsh: fork failed\r\n");
        if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
        if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }
        return -1;
    }

    if fork_result == 0 {
        // Child: wire up stdin/stdout/stderr.
        if in_fd != 0 {
            raw::raw_dup2(in_fd, 0);
            raw::raw_close(in_fd);
        }
        if out_fd != 1 {
            raw::raw_dup2(out_fd, 1);
            raw::raw_close(out_fd);
        }
        if redirect_in_fd >= 0 {
            raw::raw_dup2(redirect_in_fd, 0);
            raw::raw_close(redirect_in_fd);
        }
        if redirect_out_fd >= 0 {
            raw::raw_dup2(redirect_out_fd, 1);
            raw::raw_close(redirect_out_fd);
        }
        // Close all inherited fds >= 3 so pipe EOF propagates.
        let mut close_fd = 3i32;
        while close_fd < 64 {
            raw::raw_close(close_fd);
            close_fd += 1;
        }

        // Build null-separated argv flat buffer: "arg0\0arg1\0arg2\0"
        // command_tokens[0] is the command name (argv[0]).
        let mut argv_flat: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        for token in &command_tokens {
            argv_flat.extend_from_slice(token.as_bytes());
            argv_flat.push(0u8);
        }
        // exec replaces the process image.
        raw::raw_exec(path.as_ptr(), path.len(), argv_flat.as_ptr(), argv_flat.len());

        // exec failed.
        write_stderr(&command_tokens[0]);
        write_stderr(": command not found\r\n");
        raw::raw_exit(127);
    }

    // Parent: close fds handed to the child.
    if redirect_in_fd >= 0  { raw::raw_close(redirect_in_fd); }
    if redirect_out_fd >= 0 { raw::raw_close(redirect_out_fd); }

    fork_result as i32
}

// ---------------------------------------------------------------------------
// Pipeline execution
// ---------------------------------------------------------------------------

/// Execute a linear pipeline of one or more stages.
fn execute_pipeline(stages: &[String]) {
    let n = stages.len();
    if n == 0 {
        return;
    }

    // Create n-1 pipes: pipes[i] connects stage i to stage i+1.
    // pipes[i][0] = read end, pipes[i][1] = write end.
    let mut pipe_fds: Vec<[i32; 2]> = Vec::with_capacity(n - 1);
    for _ in 0..n.saturating_sub(1) {
        let mut fd_pair = [0i32; 2];
        if raw::raw_pipe(fd_pair.as_mut_ptr()) < 0 {
            write_stderr("bzsh: pipe creation failed\r\n");
            return;
        }
        pipe_fds.push(fd_pair);
    }

    let mut pids: Vec<i32> = Vec::with_capacity(n);

    for (index, stage) in stages.iter().enumerate() {
        let in_fd  = if index == 0     { 0 } else { pipe_fds[index - 1][0] };
        let out_fd = if index == n - 1 { 1 } else { pipe_fds[index][1] };

        let pid = execute_stage(stage, in_fd, out_fd);
        pids.push(pid);

        // Parent closes the pipe ends just passed to the child.
        if index > 0 {
            raw::raw_close(pipe_fds[index - 1][0]);
        }
        if index < n - 1 {
            raw::raw_close(pipe_fds[index][1]);
        }
    }

    // Forward SIGINT to the last (foreground) stage while the shell waits.
    let last_pid = *pids.last().unwrap_or(&-1);
    if last_pid >= 0 {
        raw::raw_setfgpid(last_pid);
    }

    // Wait for all children (reverse order avoids deadlock on full pipes).
    for pid in pids.iter().rev() {
        if *pid >= 0 {
            let mut status = 0i32;
            raw::raw_wait(*pid, &mut status as *mut i32);
        }
    }

    // Shell reclaims the terminal.
    raw::raw_setfgpid(0);
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

fn shell_main() {
    write_stdout("Bazzulto Shell\r\n");

    loop {
        write_stdout("bazzulto> ");

        let line = read_line();
        if line.is_empty() {
            continue;
        }

        // Split into pipeline stages.
        let stages = split_pipeline(&line);

        // Single-stage fast path: check for built-ins before forking.
        if stages.len() == 1 {
            let stage = stages[0].trim();
            if stage.is_empty() {
                continue;
            }
            // Check for redirects before calling try_builtin (which won't
            // handle them). If no redirect, try built-ins first.
            let has_redirect = stage.contains('<') || stage.contains('>');
            if !has_redirect {
                let tokens = match tokenize(stage) {
                    Ok(t) => t,
                    Err(()) => {
                        write_stderr("bzsh: unterminated quote\r\n");
                        continue;
                    }
                };
                if let Some(_exit_code) = try_builtin(&tokens) {
                    continue;
                }
            }
        }

        execute_pipeline(&stages);
    }
}

// ---------------------------------------------------------------------------
// Panic / alloc-error handlers (required for no_std)
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    write_stderr("bzsh: panic\r\n");
    raw::raw_exit(1)
}

