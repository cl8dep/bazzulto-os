// sh — POSIX.1-2024 shell for Bazzulto OS
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// Architecture:
//   lexer.rs    — §2.3 Token Recognition, §2.2 Quoting
//   parser.rs   — §2.9 Shell Commands (SimpleCommand, Pipeline, CompoundList)
//   executor.rs — §2.9.1.4 Command Search and Execution, §2.7 Redirection,
//                 §2.9.2 Pipelines, §2.9.3 Lists, §2.9.4 Compound Commands
//   builtins.rs — §2.14 Special Built-In Utilities, §2.15 Regular Built-Ins
//   vars.rs     — §2.5 Parameters and Variables (VarStore, special params)

#![no_std]
#![no_main]

extern crate alloc;

mod builtins;
pub(crate) mod expand;
pub(crate) mod executor;
mod lexer;
mod parser;
pub(crate) mod vars;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use bazzulto_system::raw;
use vars::VarStore;

// ---------------------------------------------------------------------------
// I/O helpers — used across modules via pub(crate)
// ---------------------------------------------------------------------------

pub(crate) fn write_fd(fd: i32, s: &str) {
    raw::raw_write(fd, s.as_ptr(), s.len());
}

pub(crate) fn write_out(s: &str) {
    write_fd(1, s);
}

pub(crate) fn write_err(s: &str) {
    write_fd(2, s);
}

/// Exit the shell with the given status code.
///
/// Called by the expander for `${var:?}` errors (§2.6.2) and any path that
/// must terminate the process. Separated from `write_err` so the test harness
/// can substitute a panic without pulling in `bazzulto_system::raw` directly.
pub(crate) fn exit_on_error(code: i32) -> ! {
    raw::raw_exit(code)
}

// ---------------------------------------------------------------------------
// Loop control signal (§2.14 break / continue)
// ---------------------------------------------------------------------------

/// Signal emitted by `break` and `continue` to unwind the loop executor stack.
pub(crate) enum LoopSignal {
    /// No active loop control signal.
    None,
    /// `break [n]` — exit n enclosing loops.
    Break(usize),
    /// `continue [n]` — skip to next iteration of the n-th enclosing loop.
    Continue(usize),
}

// ---------------------------------------------------------------------------
// Shell state
// ---------------------------------------------------------------------------

/// Top-level shell state: variables, parameters, and runtime values.
///
/// §2.5 Parameters and Variables:
///   - Named variables live in `vars` (VarStore).
///   - Positional parameters ($1..$n) live in `positional_params`.
///   - Special parameters ($?, $$, $!, $0, $#, $@, $*, $-) are computed
///     from the fields of this struct by `vars::expand_special`.
pub(crate) struct ShellState {
    // --- §2.5.2 Special parameters ---

    /// $? — exit status of the most recently executed pipeline.
    pub last_exit_status: i32,

    /// $0 — name of the shell or script (§2.5.2, §sh invocation).
    pub shell_name: String,

    /// $$ — PID of the shell process (§2.5.2).
    pub shell_pid: u32,

    /// $! — PID of the most recent background command (§2.5.2).
    pub last_background_pid: Option<i32>,

    // --- §2.5.1 Positional parameters ---

    /// $1, $2, …, $n — positional parameters.
    pub positional_params: Vec<String>,

    // --- §2.5.3 Shell variables ---

    /// All named shell variables including exported environment variables.
    pub vars: VarStore,

    // --- §2.8.1 Interactive shell flag ---

    /// Whether the shell is interactive (stdin is a terminal, no script file).
    pub is_interactive: bool,

    // --- §2.9.2 Shell option: pipefail ---

    /// If true, the exit status of a pipeline is that of the rightmost
    /// command that returned non-zero, or 0 if all commands succeeded.
    /// Set by `set -o pipefail` (§2.14 set builtin option).
    pub pipefail: bool,

    // --- §2.14 Loop control (break / continue) ---

    /// Number of currently active loop levels (for / while / until).
    /// Used to validate `break n` and `continue n` operands.
    pub loop_depth: usize,

    /// Pending loop control signal set by `break` or `continue`.
    /// Cleared by the loop executor when it handles the signal.
    pub loop_signal: LoopSignal,

    // --- §2.14 Function return ---

    /// Number of currently active function call levels.
    pub function_depth: usize,

    /// Pending return value set by the `return` builtin.
    /// `None` when not in a `return`.
    pub return_signal: Option<i32>,

    // --- §2.14 set option flags ---

    /// -e: exit on error — exit immediately if a pipeline exits non-zero.
    pub option_errexit: bool,

    /// -u: treat unset variables as errors.
    pub option_nounset: bool,

    /// -x: trace — write expanded command to stderr before executing.
    pub option_xtrace: bool,

    /// -n: no-exec — read commands but do not execute them.
    pub option_noexec: bool,

    /// -v: verbose — write input to stderr as it is read.
    pub option_verbose: bool,

    // --- §2.9.5 Shell function definitions ---

    /// Defined shell functions: (name, serialized_body_words).
    /// Body is the serialized words vector from the compound body command.
    pub functions: Vec<(String, Vec<String>)>,

    // --- §2.6.3 Command substitution callback ---

    /// Called by the expander for `$(...)` and `` `...` `` substitutions.
    /// Implemented by executor::command_substitution; stored here to break
    /// the expand ↔ executor circular dependency.
    pub command_sub_fn: fn(cmd_text: &str, state: &mut ShellState) -> String,
}

impl ShellState {
    /// Create and fully initialize a ShellState.
    ///
    /// # Safety
    /// `envp` must be a valid null-terminated array of null-terminated strings,
    /// or null.
    pub unsafe fn init(shell_name: &str, envp: *const *const u8) -> Self {
        let mut vars = VarStore::new();

        vars.init_from_envp(envp);

        let shell_pid = raw::raw_getpid().max(0) as u32;

        if !vars.is_set("IFS") {
            let _ = vars.set("IFS", " \t\n");
        }

        if !vars.is_set("PATH") {
            let _ = vars.set("PATH", "/system/bin");
            vars.export("PATH");
        }

        if !vars.is_set("HOME") {
            let _ = vars.set("HOME", "/home/user");
            vars.export("HOME");
        }

        if !vars.is_set("PS1") {
            let _ = vars.set("PS1", "$ ");
        }

        if !vars.is_set("PS2") {
            let _ = vars.set("PS2", "> ");
        }

        if !vars.is_set("PS4") {
            let _ = vars.set("PS4", "+ ");
        }

        {
            let mut buf = [0u8; 512];
            let n = raw::raw_getcwd(buf.as_mut_ptr(), buf.len());
            if n > 1 {
                let len = (n as usize).saturating_sub(1);
                if let Ok(path) = core::str::from_utf8(&buf[..len]) {
                    let _ = vars.set("PWD", path);
                    vars.export("PWD");
                }
            }
        }

        {
            let ppid = raw::raw_getppid().max(0) as u32;
            let _ = vars.set("PPID", &vars::format_u32(ppid));
        }

        // §8.3 SHELL — path of the shell itself.
        if !vars.is_set("SHELL") {
            let _ = vars.set("SHELL", shell_name);
            vars.export("SHELL");
        }

        // §8.3 USER / LOGNAME — current username.  Default to "user".
        if !vars.is_set("USER") {
            let _ = vars.set("USER", "user");
            vars.export("USER");
        }
        if !vars.is_set("LOGNAME") {
            // LOGNAME mirrors USER per POSIX §8.3.
            let username = vars.get("USER").unwrap_or("user").to_string();
            let _ = vars.set("LOGNAME", &username);
            vars.export("LOGNAME");
        }

        // §8.3 TERM — terminal type.
        if !vars.is_set("TERM") {
            let _ = vars.set("TERM", "bazzulto");
            vars.export("TERM");
        }

        // §8.3 TMPDIR — temporary directory.
        if !vars.is_set("TMPDIR") {
            let _ = vars.set("TMPDIR", "/tmp");
            vars.export("TMPDIR");
        }

        // §8.2 TZ — timezone.  Default to America/Montevideo.
        if !vars.is_set("TZ") {
            let _ = vars.set("TZ", "America/Montevideo");
            vars.export("TZ");
        }

        // §8.3 OLDPWD — previous working directory.
        // Not meaningful at shell startup; set to empty so the variable exists.
        // cd will populate it on the first directory change.
        if !vars.is_set("OLDPWD") {
            let _ = vars.set("OLDPWD", "");
        }

        ShellState {
            last_exit_status: 0,
            shell_name: shell_name.to_string(),
            shell_pid,
            last_background_pid: None,
            positional_params: Vec::new(),
            vars,
            is_interactive: true,
            pipefail: false,
            loop_depth: 0,
            loop_signal: LoopSignal::None,
            function_depth: 0,
            return_signal: None,
            option_errexit: false,
            option_nounset: false,
            option_xtrace:  false,
            option_noexec:  false,
            option_verbose: false,
            functions: Vec::new(),
            command_sub_fn: executor::command_substitution,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);

    let args_vec = bazzulto_system::args();
    let shell_name = args_vec
        .into_iter()
        .next()
        .unwrap_or_else(|| "sh")
        .to_string();

    let mut state = unsafe { ShellState::init(&shell_name, envp) };

    shell_main(&mut state);
    raw::raw_exit(state.last_exit_status)
}

// ---------------------------------------------------------------------------
// Main REPL loop
// ---------------------------------------------------------------------------

/// Read one full line from fd. Strips the trailing '\n'.
/// Returns None on EOF.
fn read_line(fd: i32) -> Option<String> {
    let mut result = String::new();
    let mut buf = [0u8; 1];
    loop {
        let n = raw::raw_read(fd, buf.as_mut_ptr(), 1);
        if n <= 0 {
            if result.is_empty() { return None; }
            return Some(result);
        }
        let byte = buf[0];
        if byte == b'\n' { return Some(result); }
        if let Ok(ch) = core::str::from_utf8(&buf[..1]) {
            result.push_str(ch);
        }
    }
}

/// §2.7.4: For each here-document redirect in the pipeline, read body lines
/// from stdin until the terminating delimiter line is found, and store the
/// body in the `HereDoc` redirect variant replacing the delimiter string.
fn read_heredoc_bodies(list: &mut parser::CompoundList, state: &ShellState) {
    let ps2 = state.vars.get("PS2").unwrap_or("> ").to_string();
    for item in list.iter_mut() {
        for cmd in item.pipeline.commands.iter_mut() {
            for redirect in &mut cmd.redirects {
                if let parser::Redirect::HereDoc(_fd, strip_tabs, delim) = redirect {
                    let delimiter = delim.clone();
                    let strip = *strip_tabs;
                    let body = lexer::read_heredoc_body(
                        &delimiter,
                        false,
                        strip,
                        || read_line(0),
                        &ps2,
                    );
                    *delim = body;
                }
            }
        }
    }
}

fn shell_main(state: &mut ShellState) {
    write_out("Bazzulto sh\r\n");

    // Accumulated token buffer for multi-line compound commands.
    let mut pending_tokens: Vec<lexer::Token> = Vec::new();
    // Whether we are inside a multi-line compound command (awaiting completion).
    let mut in_continuation = false;

    loop {
        // Prompt: PS2 when continuing a compound command, PS1 otherwise.
        let prompt = if in_continuation {
            state.vars.get("PS2").unwrap_or("> ").to_string()
        } else {
            state.vars.get("PS1").unwrap_or("$ ").to_string()
        };
        write_err(&prompt);

        let line = match read_line(0) {
            Some(l) => l,
            None => break, // EOF
        };

        // Skip empty lines and comment lines (only when not in a continuation).
        if !in_continuation {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
        }

        // §2.3: Tokenize the new line and append to the pending buffer.
        // Append a newline token so the parser sees line boundaries.
        let new_tokens = match lexer::tokenize(&line) {
            Ok(t) => t,
            Err(e) => {
                write_err("sh: ");
                write_err(e);
                write_err("\n");
                state.last_exit_status = 2;
                pending_tokens.clear();
                in_continuation = false;
                if state.is_interactive { continue; } else { break; }
            }
        };

        if new_tokens.is_empty() && !in_continuation {
            continue;
        }

        pending_tokens.extend(new_tokens);
        // Add a newline token as line terminator for the parser.
        pending_tokens.push(lexer::Token::Newline);

        // Try to parse the accumulated token buffer.
        let list = match parser::parse_compound_list(&pending_tokens) {
            Ok(l) => l,
            Err(parser::ParseError::NeedMore) => {
                // The compound command is not yet complete — read more lines.
                in_continuation = true;
                continue;
            }
            Err(e) => {
                write_err("sh: syntax error: ");
                write_err(e.message());
                write_err("\n");
                state.last_exit_status = 2;
                pending_tokens.clear();
                in_continuation = false;
                if state.is_interactive { continue; } else { break; }
            }
        };

        // Successfully parsed — reset continuation state.
        pending_tokens.clear();
        in_continuation = false;

        if list.is_empty() {
            continue;
        }

        // §2.7.4: Read here-document bodies for any `<<` in the list.
        let mut list = list;
        read_heredoc_bodies(&mut list, state);

        // Execute and update $?.
        state.last_exit_status = executor::execute_compound_list(&list, state);
    }
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    write_err("sh: panic\r\n");
    raw::raw_exit(1)
}
