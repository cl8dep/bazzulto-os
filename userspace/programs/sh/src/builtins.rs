// sh/builtins.rs — POSIX shell built-in utilities
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//   §2.14 Special Built-In Utilities: break, colon, continue, eval, exec,
//          exit, export, readonly, return, set, shift, times, trap, unset
//   §2.15 Regular Built-Ins: cd, echo, false, pwd, true, …

extern crate alloc;

use alloc::string::{String, ToString};
use bazzulto_system::raw;

use crate::{ShellState, write_err, write_out};
use crate::vars::{is_valid_name, parse_assignment};

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Execute a built-in command.
///
/// Returns `Some(exit_code)` if handled, `None` if not a built-in.
pub fn try_builtin(args: &[String], state: &mut ShellState) -> Option<i32> {
    let cmd = args.first()?.as_str();
    match cmd {
        // §2.14 Special built-ins
        ":"         => Some(0),
        "true"      => Some(0),
        "false"     => Some(1),
        "exit"      => builtin_exit(args, state),
        "export"    => Some(builtin_export(args, state)),
        "readonly"  => Some(builtin_readonly(args, state)),
        "unset"     => Some(builtin_unset(args, state)),
        "shift"     => Some(builtin_shift(args, state)),
        "set"       => Some(builtin_set(args, state)),
        "exec"      => Some(builtin_exec(args, state)),
        "break"     => Some(builtin_break(args, state)),
        "continue"  => Some(builtin_continue(args, state)),
        "return"    => Some(builtin_return(args, state)),
        "eval"      => Some(builtin_eval(args, state)),
        "."         => Some(builtin_dot(args, state)),
        "read"      => Some(builtin_read(args, state)),
        "trap"      => Some(builtin_trap(args, state)),
        "times"     => Some(builtin_times()),
        "wait"      => Some(builtin_wait(args, state)),

        // §2.15 Regular built-ins
        "cd"        => Some(builtin_cd(args, state)),
        "pwd"       => Some(builtin_pwd(state)),
        "echo"      => Some(builtin_echo(args)),

        // Job control (not yet supported)
        "jobs" => {
            write_out("(no background jobs)\n");
            Some(0)
        }
        "bg" | "fg" => {
            write_err("sh: ");
            write_err(cmd);
            write_err(": not supported\n");
            Some(1)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// §2.14: exit [n]
// ---------------------------------------------------------------------------

fn builtin_exit(args: &[String], state: &mut ShellState) -> Option<i32> {
    let code = args.get(1)
        .and_then(|s| parse_i32(s))
        .unwrap_or(state.last_exit_status);
    raw::raw_exit(code)
}

// ---------------------------------------------------------------------------
// §2.14: export [name[=value] ...]
// ---------------------------------------------------------------------------

fn builtin_export(args: &[String], state: &mut ShellState) -> i32 {
    if args.len() == 1 {
        // export with no args: list exported variables (optional in POSIX).
        state.vars.for_each_exported(|name, value| {
            write_out("export ");
            write_out(name);
            write_out("=");
            write_out(value);
            write_out("\n");
        });
        return 0;
    }

    let mut status = 0i32;
    for arg in &args[1..] {
        let s = arg.as_str();
        if let Some((name, value)) = parse_assignment(s) {
            // export NAME=value: set and mark for export.
            if let Err(e) = state.vars.set(name, value) {
                write_err("sh: export: ");
                write_err(name);
                write_err(": ");
                write_err(e);
                write_err("\n");
                status = 1;
            } else {
                state.vars.export(name);
            }
        } else if is_valid_name(s) {
            // export NAME: mark existing variable for export.
            state.vars.export(s);
        } else {
            write_err("sh: export: `");
            write_err(s);
            write_err("': not a valid identifier\n");
            status = 1;
        }
    }
    status
}

// ---------------------------------------------------------------------------
// §2.14: readonly [name[=value] ...]
// ---------------------------------------------------------------------------

fn builtin_readonly(args: &[String], state: &mut ShellState) -> i32 {
    let mut status = 0i32;
    for arg in &args[1..] {
        let s = arg.as_str();
        if let Some((name, value)) = parse_assignment(s) {
            if let Err(e) = state.vars.set(name, value) {
                write_err("sh: readonly: ");
                write_err(name);
                write_err(": ");
                write_err(e);
                write_err("\n");
                status = 1;
            } else {
                state.vars.set_readonly(name);
            }
        } else if is_valid_name(s) {
            state.vars.set_readonly(s);
        } else {
            write_err("sh: readonly: `");
            write_err(s);
            write_err("': not a valid identifier\n");
            status = 1;
        }
    }
    status
}

// ---------------------------------------------------------------------------
// §2.14: unset [-v | -f] name ...
// ---------------------------------------------------------------------------

fn builtin_unset(args: &[String], state: &mut ShellState) -> i32 {
    // For now we only unset variables (-v is the default; -f for functions
    // is deferred until §2.9.5 Function Definition is implemented).
    let mut status = 0i32;
    let mut start = 1usize;
    if args.get(1).map(|s| s.as_str()) == Some("-v") {
        start = 2;
    } else if args.get(1).map(|s| s.as_str()) == Some("-f") {
        // Functions not yet implemented — silently succeed.
        return 0;
    }
    for arg in &args[start..] {
        if let Err(e) = state.vars.unset(arg.as_str()) {
            write_err("sh: unset: ");
            write_err(arg.as_str());
            write_err(": ");
            write_err(e);
            write_err("\n");
            status = 1;
        }
    }
    status
}

// ---------------------------------------------------------------------------
// §2.14: shift [n]
// ---------------------------------------------------------------------------

fn builtin_shift(args: &[String], state: &mut ShellState) -> i32 {
    let n = args.get(1)
        .and_then(|s| parse_i32(s))
        .unwrap_or(1) as usize;
    if n > state.positional_params.len() {
        write_err("sh: shift: shift count out of range\n");
        return 1;
    }
    state.positional_params.drain(..n);
    0
}

// ---------------------------------------------------------------------------
// §2.14: set [--] [arg ...]
// ---------------------------------------------------------------------------

/// POSIX set: with arguments, replace the positional parameters.
/// `set -- arg1 arg2` sets $1=arg1, $2=arg2, etc.
/// With no arguments (§2.14): print all variables.
/// TODO (future §2.14): handle option flags (-e, -x, -u, etc.)
fn builtin_set(args: &[String], state: &mut ShellState) -> i32 {
    if args.len() == 1 {
        // No arguments: list all shell variables.
        state.vars.for_each_exported(|name, value| {
            write_out(name);
            write_out("=");
            write_out(value);
            write_out("\n");
        });
        return 0;
    }

    let mut i = 1usize;
    let mut positional_start: Option<usize> = None;

    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--" => {
                // End of options; remaining args are positional params.
                positional_start = Some(i + 1);
                i = args.len(); // exit option loop
                continue;
            }
            "-o" | "+o" => {
                // Named option: `set -o name` / `set +o name`
                let enable = arg.starts_with('-');
                i += 1;
                if i >= args.len() {
                    // `set -o` with no name: print option state.
                    write_out("errexit  "); write_out(if state.option_errexit { "on" } else { "off" }); write_out("\n");
                    write_out("nounset  "); write_out(if state.option_nounset { "on" } else { "off" }); write_out("\n");
                    write_out("xtrace   "); write_out(if state.option_xtrace  { "on" } else { "off" }); write_out("\n");
                    write_out("noexec   "); write_out(if state.option_noexec  { "on" } else { "off" }); write_out("\n");
                    write_out("verbose  "); write_out(if state.option_verbose { "on" } else { "off" }); write_out("\n");
                    write_out("pipefail "); write_out(if state.pipefail       { "on" } else { "off" }); write_out("\n");
                    return 0;
                }
                match args[i].as_str() {
                    "errexit"  => state.option_errexit = enable,
                    "nounset"  => state.option_nounset = enable,
                    "xtrace"   => state.option_xtrace  = enable,
                    "noexec"   => state.option_noexec  = enable,
                    "verbose"  => state.option_verbose = enable,
                    "pipefail" => state.pipefail        = enable,
                    name => {
                        write_err("sh: set: illegal option name: ");
                        write_err(name);
                        write_err("\n");
                        return 1;
                    }
                }
                i += 1;
                continue;
            }
            s if s.starts_with('-') && s.len() > 1 && !s.starts_with("--") => {
                // Short flags: -euxnv etc.
                for ch in s[1..].chars() {
                    match ch {
                        'e' => state.option_errexit = true,
                        'u' => state.option_nounset = true,
                        'x' => state.option_xtrace  = true,
                        'n' => state.option_noexec  = true,
                        'v' => state.option_verbose = true,
                        _ => {} // ignore unknown
                    }
                }
                i += 1;
            }
            s if s.starts_with('+') && s.len() > 1 => {
                // Short flags disable: +euxnv etc.
                for ch in s[1..].chars() {
                    match ch {
                        'e' => state.option_errexit = false,
                        'u' => state.option_nounset = false,
                        'x' => state.option_xtrace  = false,
                        'n' => state.option_noexec  = false,
                        'v' => state.option_verbose = false,
                        _ => {}
                    }
                }
                i += 1;
            }
            _ => {
                // First non-option argument: remaining args are positional params.
                positional_start = Some(i);
                i = args.len();
            }
        }
    }

    if let Some(start) = positional_start {
        state.positional_params.clear();
        for arg in &args[start..] {
            state.positional_params.push(arg.clone());
        }
    }
    0
}

// ---------------------------------------------------------------------------
// §2.15: cd [-L | -P] [dir]
// ---------------------------------------------------------------------------

fn builtin_cd(args: &[String], state: &mut ShellState) -> i32 {
    // Skip -L / -P flags (logical/physical; we always behave physically for now).
    let mut start = 1usize;
    if let Some(flag) = args.get(1) {
        if flag.as_str() == "-L" || flag.as_str() == "-P" {
            start = 2;
        }
    }

    // §2.5.3 HOME: cd with no operand goes to $HOME.
    let path = args.get(start)
        .map(|s| s.as_str())
        .unwrap_or_else(|| state.vars.get("HOME").unwrap_or("/home/user"));

    // §2.5.3 OLDPWD: capture current PWD before changing.
    let old_pwd = state.vars.get("PWD").unwrap_or("").to_string();

    let mut chdir_buf = [0u8; 512];
    let chdir_len = path.len().min(511);
    chdir_buf[..chdir_len].copy_from_slice(&path.as_bytes()[..chdir_len]);
    let result = raw::raw_chdir(chdir_buf.as_ptr());
    if result < 0 {
        write_err("sh: cd: ");
        write_err(path);
        write_err(": No such file or directory\n");
        return 1;
    }

    // §2.5.3 OLDPWD: set to the directory we just left.
    let _ = state.vars.set("OLDPWD", &old_pwd);

    // §2.5.3 PWD: update PWD after a successful cd.
    let mut buf = [0u8; 512];
    let n = raw::raw_getcwd(buf.as_mut_ptr(), buf.len());
    if n > 1 {
        let len = (n as usize).saturating_sub(1);
        if let Ok(new_pwd) = core::str::from_utf8(&buf[..len]) {
            let _ = state.vars.set("PWD", new_pwd);
        }
    }
    0
}

// ---------------------------------------------------------------------------
// §2.15: pwd [-L | -P]
// ---------------------------------------------------------------------------

fn builtin_pwd(state: &ShellState) -> i32 {
    // -L (logical): use $PWD if set. -P (physical): use getcwd.
    // Default is -L per POSIX.
    if let Some(pwd) = state.vars.get("PWD") {
        if !pwd.is_empty() {
            write_out(pwd);
            write_out("\n");
            return 0;
        }
    }
    // Fallback: ask the kernel.
    let mut buf = [0u8; 512];
    let n = raw::raw_getcwd(buf.as_mut_ptr(), buf.len());
    if n > 1 {
        let len = (n as usize).saturating_sub(1);
        if let Ok(path) = core::str::from_utf8(&buf[..len]) {
            write_out(path);
            write_out("\n");
        }
    }
    0
}

// ---------------------------------------------------------------------------
// §2.15: echo
// ---------------------------------------------------------------------------

fn builtin_echo(args: &[String]) -> i32 {
    for (i, arg) in args[1..].iter().enumerate() {
        if i > 0 { write_out(" "); }
        write_out(arg.as_str());
    }
    write_out("\n");
    0
}

// ---------------------------------------------------------------------------
// §2.14: exec [command [arg ...]]
// ---------------------------------------------------------------------------

/// Replace the current shell process image with `command` (§2.9.1.6).
///
/// With no arguments: no-op (opens/redirects are handled by the executor before
/// calling this; the executor already applied any redirects).
///
/// With arguments: resolve `command` via $PATH and call exec(). On success this
/// never returns. On failure: write an error message and return 127 (not found)
/// or 126 (found but not executable). The shell shall then exit (§2.8.1,
/// non-interactive) or continue (interactive) — callers handle that.
fn builtin_exec(args: &[String], state: &mut ShellState) -> i32 {
    if args.len() == 1 {
        // exec with no arguments: no-op per POSIX (any redirects already applied).
        return 0;
    }

    let cmd_name = args[1].as_str();

    // Resolve path using $PATH (same logic as executor).
    let path: String = if cmd_name.contains('/') {
        cmd_name.into()
    } else {
        let path_var = state.vars.get("PATH").unwrap_or("/system/bin");
        let mut found: Option<String> = None;
        for dir in path_var.split(':') {
            if dir.is_empty() { continue; }
            let mut candidate = alloc::string::String::from(dir);
            if !candidate.ends_with('/') { candidate.push('/'); }
            candidate.push_str(cmd_name);
            let mut candidate_buf = [0u8; 512];
            let candidate_len = candidate.len().min(511);
            candidate_buf[..candidate_len].copy_from_slice(&candidate.as_bytes()[..candidate_len]);
            let fd = raw::raw_open(candidate_buf.as_ptr(), 0, 0);
            if fd >= 0 {
                raw::raw_close(fd as i32);
                found = Some(candidate);
                break;
            }
        }
        match found {
            Some(p) => p,
            None => {
                write_err("sh: exec: ");
                write_err(cmd_name);
                write_err(": command not found\n");
                return 127;
            }
        }
    };

    // Build NUL-terminated argv strings and pointer array.
    let mut argv_strings: alloc::vec::Vec<alloc::vec::Vec<u8>> = args[1..].iter().map(|w| {
        let mut s = alloc::vec::Vec::from(w.as_bytes());
        s.push(0u8);
        s
    }).collect();
    let mut argv_ptrs: alloc::vec::Vec<*const u8> =
        argv_strings.iter().map(|s| s.as_ptr()).collect();
    argv_ptrs.push(core::ptr::null());

    // §2.9.1.6: replace process image with full environment.
    let (envp_flat, envp_offsets) = state.vars.build_envp();
    let mut envp_ptrs: alloc::vec::Vec<*const u8> =
        envp_offsets.iter().map(|&off| envp_flat[off..].as_ptr()).collect();
    envp_ptrs.push(core::ptr::null());

    // NUL-terminate the path.
    let mut path_buf = [0u8; 512];
    let path_buf_len = path.len().min(511);
    path_buf[..path_buf_len].copy_from_slice(&path.as_bytes()[..path_buf_len]);

    raw::raw_exec(path_buf.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr());
    // suppress unused warnings
    let _ = &argv_strings;

    // exec failed.
    write_err("sh: exec: ");
    write_err(cmd_name);
    write_err(": cannot execute\n");
    126
}

// ---------------------------------------------------------------------------
// §2.14: break [n]
// ---------------------------------------------------------------------------

fn builtin_break(args: &[String], state: &mut ShellState) -> i32 {
    let n = args.get(1).and_then(|s| parse_i32(s)).unwrap_or(1) as usize;
    if n == 0 {
        write_err("sh: break: n must be >= 1\n");
        return 1;
    }
    if state.loop_depth == 0 {
        write_err("sh: break: not in a loop\n");
        return 1;
    }
    state.loop_signal = crate::LoopSignal::Break(n);
    0
}

// ---------------------------------------------------------------------------
// §2.14: continue [n]
// ---------------------------------------------------------------------------

fn builtin_continue(args: &[String], state: &mut ShellState) -> i32 {
    let n = args.get(1).and_then(|s| parse_i32(s)).unwrap_or(1) as usize;
    if n == 0 {
        write_err("sh: continue: n must be >= 1\n");
        return 1;
    }
    if state.loop_depth == 0 {
        write_err("sh: continue: not in a loop\n");
        return 1;
    }
    state.loop_signal = crate::LoopSignal::Continue(n);
    0
}

// ---------------------------------------------------------------------------
// §2.14: return [n]
// ---------------------------------------------------------------------------

fn builtin_return(args: &[String], state: &mut ShellState) -> i32 {
    if state.function_depth == 0 {
        write_err("sh: return: not in a function\n");
        return 1;
    }
    let n = args.get(1)
        .and_then(|s| parse_i32(s))
        .unwrap_or(state.last_exit_status);
    state.return_signal = Some(n);
    n
}

// ---------------------------------------------------------------------------
// §2.14: eval [arg ...]
// ---------------------------------------------------------------------------

fn builtin_eval(args: &[String], state: &mut ShellState) -> i32 {
    use alloc::string::ToString;
    // Join args[1..] with spaces into a single string.
    let mut input = alloc::string::String::new();
    for (i, arg) in args[1..].iter().enumerate() {
        if i > 0 { input.push(' '); }
        input.push_str(arg.as_str());
    }
    if input.is_empty() { return 0; }

    let tokens = match crate::lexer::tokenize(&input) {
        Ok(t)  => t,
        Err(e) => {
            write_err("sh: eval: ");
            write_err(e);
            write_err("\n");
            return 1;
        }
    };
    match crate::parser::parse_compound_list(&tokens) {
        Ok(list) => crate::executor::execute_compound_list(&list, state),
        Err(e)   => {
            write_err("sh: eval: syntax error: ");
            write_err(e.message());
            write_err("\n");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// §2.14: . file [arg ...]
// ---------------------------------------------------------------------------

fn builtin_dot(args: &[String], state: &mut ShellState) -> i32 {
    let filename = match args.get(1) {
        Some(f) => f.as_str(),
        None => {
            write_err("sh: .: missing operand\n");
            return 2;
        }
    };

    // Resolve via PATH if not an absolute or relative path.
    let path: alloc::string::String = if filename.contains('/') {
        filename.into()
    } else {
        let path_var = state.vars.get("PATH").unwrap_or("/system/bin");
        let mut found: Option<alloc::string::String> = None;
        for dir in path_var.split(':') {
            if dir.is_empty() { continue; }
            let mut candidate = alloc::string::String::from(dir);
            if !candidate.ends_with('/') { candidate.push('/'); }
            candidate.push_str(filename);
            let mut dot_candidate_buf = [0u8; 512];
            let dot_candidate_len = candidate.len().min(511);
            dot_candidate_buf[..dot_candidate_len].copy_from_slice(&candidate.as_bytes()[..dot_candidate_len]);
            let fd = raw::raw_open(dot_candidate_buf.as_ptr(), 0, 0);
            if fd >= 0 {
                raw::raw_close(fd as i32);
                found = Some(candidate);
                break;
            }
        }
        match found {
            Some(p) => p,
            None => {
                write_err("sh: .: ");
                write_err(filename);
                write_err(": not found\n");
                return 127;
            }
        }
    };

    let mut dot_path_buf = [0u8; 512];
    let dot_path_len = path.len().min(511);
    dot_path_buf[..dot_path_len].copy_from_slice(&path.as_bytes()[..dot_path_len]);
    let fd = raw::raw_open(dot_path_buf.as_ptr(), 0, 0);
    if fd < 0 {
        write_err("sh: .: ");
        write_err(filename);
        write_err(": cannot open\n");
        return 1;
    }
    let fd = fd as i32;

    // Optional positional parameters from the remaining args.
    if args.len() > 2 {
        state.positional_params.clear();
        for arg in &args[2..] {
            state.positional_params.push(arg.clone());
        }
    }

    // Read and execute lines from the file.
    // `read_line_from_fd` returns `(line, eof)`.
    fn read_line_from_fd(fd: i32) -> (alloc::string::String, bool) {
        let mut line = alloc::string::String::new();
        let mut buf = [0u8; 1];
        loop {
            let n = raw::raw_read(fd, buf.as_mut_ptr(), 1);
            if n <= 0 { return (line, true); } // EOF or error
            let byte = buf[0];
            if byte == b'\n' { return (line, false); }
            if let Ok(s) = core::str::from_utf8(&[byte]) {
                line.push_str(s);
            }
        }
    }

    let mut last_status = 0i32;
    let mut pending_tokens: alloc::vec::Vec<crate::lexer::Token> = alloc::vec::Vec::new();

    loop {
        let (line, eof) = read_line_from_fd(fd);

        if eof && line.is_empty() && pending_tokens.is_empty() {
            break;
        }

        let new_tokens = match crate::lexer::tokenize(&line) {
            Ok(t)  => t,
            Err(_) => {
                if eof { break; }
                continue;
            }
        };
        pending_tokens.extend(new_tokens);
        pending_tokens.push(crate::lexer::Token::Newline);

        match crate::parser::parse_compound_list(&pending_tokens) {
            Ok(list) => {
                pending_tokens.clear();
                if !list.is_empty() {
                    last_status = crate::executor::execute_compound_list(&list, state);
                }
            }
            Err(crate::parser::ParseError::NeedMore) => {
                if eof {
                    // EOF inside a compound command: discard.
                    pending_tokens.clear();
                    break;
                }
                // Otherwise, read more lines.
            }
            Err(_) => {
                pending_tokens.clear();
            }
        }

        if eof { break; }
    }

    raw::raw_close(fd);
    last_status
}

// ---------------------------------------------------------------------------
// §2.15: read [-r] [name ...]
// ---------------------------------------------------------------------------

fn builtin_read(args: &[String], state: &mut ShellState) -> i32 {
    // Parse -r flag.
    let mut raw_mode = false;
    let mut var_start = 1usize;
    if args.get(1).map(|s| s.as_str()) == Some("-r") {
        raw_mode = true;
        var_start = 2;
    }

    // Read one line from stdin.
    let mut line = alloc::string::String::new();
    let mut buf = [0u8; 1];
    loop {
        let n = raw::raw_read(0, buf.as_mut_ptr(), 1);
        if n <= 0 {
            // EOF: return 1 to signal end of input.
            if line.is_empty() { return 1; }
            break;
        }
        let byte = buf[0];
        if byte == b'\n' { break; }

        // §2.15 read: backslash-newline continuation unless -r.
        if byte == b'\\' && !raw_mode {
            let n2 = raw::raw_read(0, buf.as_mut_ptr(), 1);
            if n2 > 0 && buf[0] == b'\n' {
                // Line continuation: read next line.
                continue;
            } else {
                // Not a continuation: push backslash then the next char.
                line.push('\\');
                if n2 > 0 {
                    if let Ok(s) = core::str::from_utf8(&[buf[0]]) {
                        line.push_str(s);
                    }
                }
                continue;
            }
        }

        if let Ok(s) = core::str::from_utf8(&[byte]) {
            line.push_str(s);
        }
    }

    let var_names: alloc::vec::Vec<&str> = args[var_start..].iter()
        .map(|s| s.as_str())
        .collect();

    if var_names.is_empty() {
        // No variable names: set REPLY.
        let _ = state.vars.set("REPLY", &line);
        return 0;
    }

    // Split by IFS and assign to variables (clone to avoid borrow conflict with set).
    let ifs: alloc::string::String = state.vars.get("IFS").unwrap_or(" \t\n").to_string();
    let mut fields: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    if ifs.is_empty() {
        fields.push(line.clone());
    } else {
        let mut current = alloc::string::String::new();
        for ch in line.chars() {
            if ifs.contains(ch) {
                if !current.is_empty() {
                    fields.push(current.clone());
                    current.clear();
                }
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            fields.push(current);
        }
    }

    // Assign fields to variables. Last variable gets the remainder.
    for (idx, name) in var_names.iter().enumerate() {
        let value = if idx + 1 < var_names.len() {
            fields.get(idx).cloned().unwrap_or_default()
        } else {
            // Last variable: join remaining fields.
            fields[idx..].join(ifs.chars().next().map(|c| c.to_string()).unwrap_or(" ".into()).as_str())
        };
        let _ = state.vars.set(name, &value);
    }

    0
}

// ---------------------------------------------------------------------------
// §2.14: trap [action signal ...]
// ---------------------------------------------------------------------------

fn builtin_trap(args: &[String], state: &mut ShellState) -> i32 {
    // Minimal implementation: signal ignore and reset.
    // Full handler execution (async string eval on signal) deferred.
    if args.len() < 2 {
        // trap with no args: print current traps — not yet tracked.
        return 0;
    }

    let action = &args[1];

    if action == "-" {
        // Reset signals to default (SIG_DFL = 0 at sa_handler offset 0).
        // rt_sigaction struct: {sa_handler:u64, sa_flags:u64, sa_restorer:u64, sa_mask:[u64;16]}
        // = 3*8 + 16*8 = 152 bytes total.
        let mut sigact_default = [0u8; 152];
        // sa_handler = 0 (SIG_DFL) — already zero-initialized.
        for sig_name in &args[2..] {
            let sig = signal_name_to_number(sig_name.as_str());
            if sig > 0 {
                raw::raw_sigaction(sig, sigact_default.as_ptr(), core::ptr::null_mut());
            }
        }
        return 0;
    }

    if action.is_empty() {
        // Ignore signals (SIG_IGN = 1 at sa_handler offset 0).
        let mut sigact_ignore = [0u8; 152];
        // sa_handler = 1 (SIG_IGN) stored as little-endian u64 at offset 0.
        sigact_ignore[0] = 1u8;
        for sig_name in &args[2..] {
            let sig = signal_name_to_number(sig_name.as_str());
            if sig > 0 {
                raw::raw_sigaction(sig, sigact_ignore.as_ptr(), core::ptr::null_mut());
            }
        }
        return 0;
    }

    // Store handler string (future: evaluate on signal delivery).
    // For now, ignore handler strings — just acknowledge.
    let _ = state;
    0
}

fn signal_name_to_number(name: &str) -> i32 {
    match name {
        "0" | "EXIT"   =>  0,
        "1" | "HUP"    =>  1,
        "2" | "INT"    =>  2,
        "3" | "QUIT"   =>  3,
        "6" | "ABRT"   =>  6,
        "9" | "KILL"   =>  9,
        "14" | "ALRM"  => 14,
        "15" | "TERM"  => 15,
        _ => -1,
    }
}

// ---------------------------------------------------------------------------
// §2.14: times
// ---------------------------------------------------------------------------

fn builtin_times() -> i32 {
    // No getrusage syscall coverage yet — output zeroes.
    // POSIX format: "<user-shell> <sys-shell>\n<user-children> <sys-children>"
    write_out("0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n");
    0
}

// ---------------------------------------------------------------------------
// §2.15: wait [pid ...]
// ---------------------------------------------------------------------------

fn builtin_wait(args: &[String], state: &mut ShellState) -> i32 {
    if args.len() == 1 {
        // No args: wait for the most recent background process.
        if let Some(pid) = state.last_background_pid {
            let mut raw_status = 0i32;
            raw::raw_wait(pid, &mut raw_status as *mut i32, 0);
        }
        return 0;
    }

    let mut last_status = 0i32;
    for arg in &args[1..] {
        if let Some(pid) = parse_i32(arg) {
            let mut raw_status = 0i32;
            raw::raw_wait(pid, &mut raw_status as *mut i32, 0);
            last_status = ((raw_status as u32) & 0xFF) as i32;
        }
    }
    last_status
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_i32(s: &str) -> Option<i32> {
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
