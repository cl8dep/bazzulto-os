// sh/executor.rs — POSIX §2.9 Shell Commands execution
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// §2.9.1   Simple Commands: assignments, words, redirects
// §2.9.1.2 Variable Assignments: scoping rules
// §2.9.1.3 No Command Name
// §2.9.1.4 Command Search and Execution order
// §2.9.1.6 Non-built-in Utility Execution + ENOEXEC heuristic
// §2.9.2   Pipelines: `!` negation; pipefail exit status
// §2.9.3   Lists: &&, ||, ;, & (background)
// §2.9.4.1 Grouping: ( subshell ) and { group ; }
// §2.9.4.2 For loop: for name [in words]; do body; done
// §2.7     Redirection (all 8 operators)
// §2.8.2   Exit Status: 127 = not found, 126 = not executable

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::String;
use bazzulto_system::raw;

use crate::parser::{
    CompoundList, Pipeline, SimpleCommand, Redirect, Separator,
    TAG_SUBSHELL, TAG_GROUP, TAG_FOR, TAG_IF, TAG_WHILE, TAG_UNTIL, TAG_CASE,
    TAG_BODY, TAG_THEN, TAG_ELIF, TAG_ELSE, TAG_FI, TAG_CASE_ITEM, TAG_ESAC,
    TAG_FUNCDEF,
    deserialize_list,
};
use crate::{ShellState, write_err};
use crate::builtins::try_builtin;
use crate::expand::{expand_word, expand_word_nosplit, pattern_matches};
use crate::vars::parse_assignment;

// ---------------------------------------------------------------------------
// §2.9.3 AND-OR list: && and ||
// ---------------------------------------------------------------------------
//
// AND-OR evaluation is handled at the list level. The `CompoundList` is a
// flat array of `AndOrItem`s where each item's `separator` describes the
// *operator that follows it*. We walk the list in order:
//
//   - After `&&`: execute next only if last_status == 0
//   - After `||`: execute next only if last_status != 0
//   - After `;` / end: always execute next
//   - After `&`: already async; always continue

/// Execute a `CompoundList` respecting AND/OR short-circuit evaluation.
pub fn execute_compound_list(list: &CompoundList, state: &mut ShellState) -> i32 {
    let mut last_status = 0i32;
    let mut skip_next = false;

    let n = list.len();
    for (idx, item) in list.iter().enumerate() {
        if skip_next {
            skip_next = false;
            // Still need to check whether *this* item's separator means we
            // skip or run the one after it.
            // Actually we just need to propagate the skip correctly.
            // For `A && B || C`: if A fails, skip B (&&), run C (||).
            // We propagate by checking the current item's separator against
            // the (already updated) last_status.
            // Since we skipped running this item, last_status is unchanged;
            // so we re-evaluate the skip for the next one.
            let sep = &item.separator;
            match sep {
                Separator::And => { if last_status != 0 { skip_next = true; } }
                Separator::Or  => { if last_status == 0 { skip_next = true; } }
                _ => {}
            }
            continue;
        }

        // §2.14 -n: read commands but do not execute.
        if state.option_noexec {
            continue;
        }

        let status = if item.separator == Separator::Amp {
            execute_pipeline_async(&item.pipeline, state)
        } else {
            execute_pipeline_sync(&item.pipeline, state)
        };

        last_status = status;
        state.last_exit_status = last_status;

        // Propagate return / break / continue signals immediately.
        if state.return_signal.is_some() {
            return last_status;
        }
        if !matches!(state.loop_signal, crate::LoopSignal::None) {
            return last_status;
        }

        // §2.14 -e: exit on error.
        // Do not apply to compound condition-test positions (if/while cond) —
        // those are guarded by the caller. Apply here for top-level statements
        // and non-conditional and-or items.
        if state.option_errexit && last_status != 0 {
            // Only exit if not part of an AND/OR chain that can still recover.
            match &item.separator {
                Separator::And | Separator::Or => {} // skip: next item may still run
                _ => {
                    crate::exit_on_error(last_status);
                }
            }
        }

        // Determine whether to skip the next item.
        if idx + 1 < n {
            match &item.separator {
                Separator::And => { if last_status != 0 { skip_next = true; } }
                Separator::Or  => { if last_status == 0 { skip_next = true; } }
                _ => {}
            }
        }
    }

    last_status
}

// ---------------------------------------------------------------------------
// §2.9.2 Pipeline execution
// ---------------------------------------------------------------------------

/// Execute a pipeline synchronously (foreground). Returns exit status.
pub fn execute_pipeline_sync(pipeline: &Pipeline, state: &mut ShellState) -> i32 {
    let status = run_pipeline(pipeline, state, false);
    // §2.9.2: `!` negates the exit status.
    if pipeline.negate {
        if status == 0 { 1 } else { 0 }
    } else {
        status
    }
}

/// Execute a pipeline asynchronously (background `&`). Returns 0.
///
/// §2.9.3: The shell shall not wait for the pipeline to complete.
/// `$!` is set to the PID of the last command in the pipeline (§2.5.2).
pub fn execute_pipeline_async(pipeline: &Pipeline, state: &mut ShellState) -> i32 {
    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        return 1;
    }
    if pid == 0 {
        // Child: run the pipeline in a new process group and exit.
        let status = run_pipeline(pipeline, state, false);
        raw::raw_exit(status);
    }
    // Parent: record $! and return 0.
    state.last_background_pid = Some(pid as i32);
    0
}

/// Inner pipeline runner shared by sync and async paths.
///
/// `state.pipefail`: if set, the exit status of a multi-stage pipeline is
/// that of the rightmost command that failed (non-zero), or 0 if all succeeded.
fn run_pipeline(pipeline: &Pipeline, state: &mut ShellState, _in_subshell: bool) -> i32 {
    let commands = &pipeline.commands;
    let n = commands.len();

    if n == 0 { return 0; }

    // Single command fast path.
    if n == 1 {
        return execute_command(&commands[0], 0, 1, state);
    }

    // Multi-stage pipeline: create n-1 pipes.
    let mut pipe_fds: Vec<[i32; 2]> = Vec::with_capacity(n - 1);
    for _ in 0..n - 1 {
        let mut fd_pair = [0i32; 2];
        if raw::raw_pipe(fd_pair.as_mut_ptr()) < 0 {
            write_err("sh: pipe: failed to create pipe\n");
            return 1;
        }
        pipe_fds.push(fd_pair);
    }

    let mut pids: Vec<i32> = Vec::with_capacity(n);
    let mut statuses: Vec<i32> = Vec::with_capacity(n);

    for (index, cmd) in commands.iter().enumerate() {
        let in_fd  = if index == 0     { 0 }                       else { pipe_fds[index - 1][0] };
        let out_fd = if index == n - 1 { 1 }                       else { pipe_fds[index][1] };

        let mut expanded_words: Vec<String> = Vec::new();
        for word in &cmd.words {
            let fields = expand_word(word.as_str(), state);
            expanded_words.extend(fields);
        }
        let mut stage_assignments: Vec<(String, String)> = Vec::new();
        for raw_assign in &cmd.assignments {
            if let Some((name, raw_value)) = parse_assignment(raw_assign.as_str()) {
                let expanded_value = expand_word_nosplit(raw_value, state);
                stage_assignments.push((name.into(), expanded_value));
            }
        }
        let expanded_redirects: Vec<Redirect> = cmd.redirects.iter()
            .map(|r| expand_redirect(r, state))
            .collect();
        let expanded_cmd = SimpleCommand {
            assignments: Vec::new(),
            words: expanded_words,
            redirects: expanded_redirects,
        };

        let pid = fork_and_exec_expanded(&expanded_cmd, in_fd, out_fd, &stage_assignments, state);
        pids.push(pid);

        if index > 0 { raw::raw_close(pipe_fds[index - 1][0]); }
        if index < n - 1 { raw::raw_close(pipe_fds[index][1]); }
    }

    let last_pid = *pids.last().unwrap_or(&-1);
    if last_pid >= 0 { raw::raw_setfgpid(last_pid); }

    let mut last_status = 0i32;
    let mut any_failed  = false;
    for (i, pid) in pids.iter().enumerate() {
        if *pid >= 0 {
            let mut raw_status = 0i32;
            raw::raw_wait(*pid, &mut raw_status as *mut i32);
            let s = decode_wait_status(raw_status);
            statuses.push(s);
            if i == n - 1 { last_status = s; }
            if s != 0 { any_failed = true; }
        } else {
            statuses.push(0);
        }
    }

    raw::raw_setfgpid(0);

    // §2.9.2 pipefail option: if set, the exit status is that of the
    // rightmost command that failed, or 0 if all commands succeeded.
    if state.pipefail && any_failed {
        // Return the exit status of the rightmost failed command.
        for s in statuses.iter().rev() {
            if *s != 0 { return *s; }
        }
    }

    last_status
}

// ---------------------------------------------------------------------------
// Single command execution
// ---------------------------------------------------------------------------

/// Execute one simple command with the given stdin/stdout fds.
///
/// Dispatches compound commands (subshell, group, for) when detected by the
/// synthetic word tag in `words[0]`.
fn execute_command(cmd: &SimpleCommand, in_fd: i32, out_fd: i32, state: &mut ShellState) -> i32 {
    // Detect compound-command tags.
    if let Some(tag) = cmd.words.first().map(|w| w.as_str()) {
        if tag == TAG_SUBSHELL { return execute_subshell(cmd, in_fd, out_fd, state); }
        if tag == TAG_GROUP    { return execute_group(cmd, in_fd, out_fd, state); }
        if tag == TAG_FOR      { return execute_for(cmd, in_fd, out_fd, state); }
        if tag == TAG_IF       { return execute_if(cmd, in_fd, out_fd, state); }
        if tag == TAG_WHILE    { return execute_while(cmd, in_fd, out_fd, state, false); }
        if tag == TAG_UNTIL    { return execute_while(cmd, in_fd, out_fd, state, true); }
        if tag == TAG_CASE     { return execute_case(cmd, in_fd, out_fd, state); }
        if tag == TAG_FUNCDEF  { return execute_funcdef(cmd, state); }
    }

    // §2.9.1.1 step 4: expand assignments.
    let mut assignments: Vec<(String, String)> = Vec::new();
    for raw_assign in &cmd.assignments {
        if let Some((name, raw_value)) = parse_assignment(raw_assign.as_str()) {
            let expanded_value = expand_word_nosplit(raw_value, state);
            assignments.push((name.into(), expanded_value));
        }
    }

    // §2.9.1.1 step 2: expand words.
    let mut expanded_words: Vec<String> = Vec::new();
    for word in &cmd.words {
        let fields = expand_word(word.as_str(), state);
        expanded_words.extend(fields);
    }

    // §2.7: expand redirect filenames.
    let expanded_redirects: Vec<Redirect> = cmd.redirects.iter()
        .map(|r| expand_redirect(r, state))
        .collect();

    // §2.14 -x: trace — print PS4 + expanded command to stderr.
    if state.option_xtrace && !expanded_words.is_empty() {
        let ps4 = state.vars.get("PS4").map(String::from).unwrap_or_else(|| "+ ".into());
        write_err(ps4.as_str());
        for (i, w) in expanded_words.iter().enumerate() {
            if i > 0 { write_err(" "); }
            write_err(w.as_str());
        }
        write_err("\n");
    }

    // §2.9.1.3: No command name after expansion.
    if expanded_words.is_empty() {
        return execute_no_command(&assignments, &expanded_redirects, in_fd, out_fd, state);
    }

    let cmd_name: String = expanded_words[0].clone();

    // §2.9.1.2: Special built-ins — assignments affect the current env.
    if is_special_builtin(cmd_name.as_str()) {
        for (name, value) in &assignments {
            if let Err(e) = state.vars.set(name.as_str(), value.as_str()) {
                write_err("sh: ");
                write_err(name.as_str());
                write_err("=: ");
                write_err(e);
                write_err("\n");
                return 1;
            }
        }
        let expanded_cmd = SimpleCommand {
            assignments: Vec::new(),
            words:       expanded_words,
            redirects:   expanded_redirects,
        };
        return run_builtin_with_io(&expanded_cmd, in_fd, out_fd, &[], state);
    }

    let expanded_cmd = SimpleCommand {
        assignments: Vec::new(),
        words:       expanded_words,
        redirects:   expanded_redirects,
    };

    // §2.9.1.4 step 1c: Shell functions.
    let func_body_words = state.functions.iter()
        .find(|(n, _)| n.as_str() == cmd_name.as_str())
        .map(|(_, w)| w.clone());
    if let Some(body_words) = func_body_words {
        return execute_function_call(
            &body_words, &expanded_cmd.words[1..], &expanded_cmd.redirects,
            in_fd, out_fd, &assignments, state,
        );
    }

    // §2.9.1.4 step 1d: Regular built-ins.
    if let Some(builtin_result) = try_builtin(&expanded_cmd.words, state) {
        let has_io_change = in_fd != 0 || out_fd != 1 || !expanded_cmd.redirects.is_empty();
        if !has_io_change && assignments.is_empty() {
            return builtin_result;
        }
        return run_builtin_with_io(&expanded_cmd, in_fd, out_fd, &assignments, state);
    }

    // §2.9.1.4 step 1e / §2.9.1.6: External command via PATH search + exec.
    fork_and_exec_expanded(&expanded_cmd, in_fd, out_fd, &assignments, state)
}

/// Run a built-in with I/O redirection by forking a child.
fn run_builtin_with_io(
    cmd: &SimpleCommand,
    in_fd: i32,
    out_fd: i32,
    assignments: &[(String, String)],
    state: &mut ShellState,
) -> i32 {
    let redirect_fds = match open_redirects(&cmd.redirects) {
        Some(fds) => fds,
        None => return 1,
    };
    let (redir_in, redir_out) = redirect_fds;

    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        close_optional(redir_in);
        close_optional(redir_out);
        return 1;
    }
    if pid == 0 {
        for (name, value) in assignments {
            let _ = state.vars.set(name.as_str(), value.as_str());
            state.vars.export(name.as_str());
        }
        apply_io(in_fd, out_fd, redir_in, redir_out);
        let code = try_builtin(&cmd.words, state).unwrap_or(0);
        raw::raw_exit(code);
    }
    close_optional(redir_in);
    close_optional(redir_out);
    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    decode_wait_status(raw_status)
}

/// §2.9.1.3: Execute a simple command that has no command name.
fn execute_no_command(
    assignments: &[(String, String)],
    redirects: &[Redirect],
    in_fd: i32,
    out_fd: i32,
    state: &mut ShellState,
) -> i32 {
    for (name, value) in assignments {
        if let Err(e) = state.vars.set(name.as_str(), value.as_str()) {
            write_err("sh: ");
            write_err(name.as_str());
            write_err("=: ");
            write_err(e);
            write_err("\n");
            return 1;
        }
    }

    if redirects.is_empty() { return 0; }

    let redirect_fds = match open_redirects(redirects) {
        Some(fds) => fds,
        None => return 1,
    };
    let (redir_in, redir_out) = redirect_fds;

    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        close_optional(redir_in);
        close_optional(redir_out);
        return 1;
    }
    if pid == 0 {
        apply_io(in_fd, out_fd, redir_in, redir_out);
        raw::raw_exit(0);
    }
    close_optional(redir_in);
    close_optional(redir_out);
    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    decode_wait_status(raw_status)
}

// ---------------------------------------------------------------------------
// §2.9.4.1 Subshell and Group
// ---------------------------------------------------------------------------

/// Execute a `( compound-list )` subshell.
///
/// §2.9.4.1: The compound-list is executed in a subshell environment — all
/// variable assignments and shell option changes are local to the subshell.
fn execute_subshell(cmd: &SimpleCommand, in_fd: i32, out_fd: i32, state: &mut ShellState) -> i32 {
    let (list, _) = deserialize_list(&cmd.words, 1); // skip TAG_SUBSHELL

    let redirect_fds = match open_redirects(&cmd.redirects) {
        Some(fds) => fds,
        None => return 1,
    };
    let (redir_in, redir_out) = redirect_fds;

    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        close_optional(redir_in);
        close_optional(redir_out);
        return 1;
    }
    if pid == 0 {
        apply_io(in_fd, out_fd, redir_in, redir_out);
        let status = execute_compound_list(&list, state);
        raw::raw_exit(status);
    }
    close_optional(redir_in);
    close_optional(redir_out);
    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    decode_wait_status(raw_status)
}

/// Execute a `{ compound-list ; }` group in the current shell environment.
///
/// §2.9.4.1: Unlike a subshell, a group command executes in the current
/// environment. Variable assignments and cd affect the current shell.
fn execute_group(cmd: &SimpleCommand, in_fd: i32, out_fd: i32, state: &mut ShellState) -> i32 {
    let (list, _) = deserialize_list(&cmd.words, 1); // skip TAG_GROUP

    // If there are redirections, we must fork to isolate I/O without losing state.
    // Without redirections, run directly in the current shell.
    let has_redir = !cmd.redirects.is_empty() || in_fd != 0 || out_fd != 1;

    if !has_redir {
        return execute_compound_list(&list, state);
    }

    let redirect_fds = match open_redirects(&cmd.redirects) {
        Some(fds) => fds,
        None => return 1,
    };
    let (redir_in, redir_out) = redirect_fds;

    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        close_optional(redir_in);
        close_optional(redir_out);
        return 1;
    }
    if pid == 0 {
        apply_io(in_fd, out_fd, redir_in, redir_out);
        let status = execute_compound_list(&list, state);
        raw::raw_exit(status);
    }
    close_optional(redir_in);
    close_optional(redir_out);
    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    decode_wait_status(raw_status)
}

// ---------------------------------------------------------------------------
// §2.9.4.2 For loop
// ---------------------------------------------------------------------------

/// Execute a `for name [in words]; do body; done` loop.
///
/// §2.9.4.2: For each word in the word list (or in `"$@"` if omitted),
/// assign it to the loop variable and execute the body.
fn execute_for(cmd: &SimpleCommand, _in_fd: i32, _out_fd: i32, state: &mut ShellState) -> i32 {
    // Deserialize: words[0] = TAG_FOR, words[1] = variable, words[2] = "none" | "some:N",
    // words[3..3+N] = word list, then TAG_BODY, then body list.
    let words = &cmd.words;
    let mut i = 1usize;

    let variable = words.get(i).cloned().unwrap_or_default();
    i += 1;

    let word_spec = words.get(i).map(|s| s.as_str()).unwrap_or("none");
    i += 1;

    let iteration_words: Vec<String> = if word_spec == "none" {
        // §2.9.4.2: If no `in` clause, iterate over positional parameters.
        state.positional_params.clone()
    } else if let Some(count_str) = word_spec.strip_prefix("some:") {
        let count = count_str.parse::<usize>().unwrap_or(0);
        let mut raw_words: Vec<String> = Vec::new();
        for _ in 0..count {
            raw_words.push(words.get(i).cloned().unwrap_or_default());
            i += 1;
        }
        // §2.9.4.2: Each word in the list is expanded before use.
        let mut expanded: Vec<String> = Vec::new();
        for w in &raw_words {
            expanded.extend(expand_word(w.as_str(), state));
        }
        expanded
    } else {
        Vec::new()
    };

    // Expect TAG_BODY sentinel.
    if words.get(i).map(|s| s.as_str()) == Some(TAG_BODY) {
        i += 1;
    }

    let (body, _) = deserialize_list(words, i);

    state.loop_depth += 1;
    let mut last_status = 0i32;
    for word in &iteration_words {
        // §2.9.4.2: Set the loop variable in the current environment.
        let _ = state.vars.set(variable.as_str(), word.as_str());

        last_status = execute_compound_list(&body, state);

        // Check for break/continue signals.
        match &state.loop_signal {
            crate::LoopSignal::Break(1) => {
                state.loop_signal = crate::LoopSignal::None;
                break;
            }
            crate::LoopSignal::Break(n) if *n > 1 => {
                let n = n - 1;
                state.loop_signal = crate::LoopSignal::Break(n);
                break;
            }
            crate::LoopSignal::Continue(1) => {
                state.loop_signal = crate::LoopSignal::None;
                // continue to next iteration
            }
            crate::LoopSignal::Continue(n) if *n > 1 => {
                let n = n - 1;
                state.loop_signal = crate::LoopSignal::Continue(n);
                break;
            }
            _ => {}
        }
    }
    state.loop_depth -= 1;

    last_status
}

// ---------------------------------------------------------------------------
// §2.9.4.4 If conditional
// ---------------------------------------------------------------------------

/// Execute an `if cond; then body; [elif cond; then body;] [else body;] fi` command.
///
/// Serialization consumed (words[0] = TAG_IF already checked):
///   TAG_IF <cond-list> TAG_THEN <then-list>
///   [TAG_ELIF <cond-list> TAG_THEN <then-list>] ...
///   [TAG_ELSE <else-list>]
///   TAG_FI
fn execute_if(cmd: &SimpleCommand, _in_fd: i32, _out_fd: i32, state: &mut ShellState) -> i32 {
    let words = &cmd.words;
    let mut i = 1usize; // skip TAG_IF

    // Parse and evaluate condition.
    let (cond_list, consumed) = deserialize_list(words, i);
    i += consumed;

    // Expect TAG_THEN.
    if words.get(i).map(|s| s.as_str()) == Some(TAG_THEN) { i += 1; }

    // Evaluate condition.
    let cond_status = execute_compound_list(&cond_list, state);

    if cond_status == 0 {
        // Condition succeeded — execute the then-body.
        let (then_list, _) = deserialize_list(words, i);
        return execute_compound_list(&then_list, state);
    }

    // Skip the then-body: find TAG_ELIF / TAG_ELSE / TAG_FI.
    let (_, consumed) = deserialize_list(words, i);
    i += consumed;

    // Walk elif clauses.
    loop {
        match words.get(i).map(|s| s.as_str()) {
            Some(TAG_FI) | None => return 0,
            Some(TAG_ELSE) => {
                i += 1; // skip TAG_ELSE
                let (else_list, _) = deserialize_list(words, i);
                return execute_compound_list(&else_list, state);
            }
            Some(TAG_ELIF) => {
                i += 1; // skip TAG_ELIF
                let (elif_cond, consumed) = deserialize_list(words, i);
                i += consumed;
                if words.get(i).map(|s| s.as_str()) == Some(TAG_THEN) { i += 1; }
                let elif_status = execute_compound_list(&elif_cond, state);
                if elif_status == 0 {
                    let (elif_body, _) = deserialize_list(words, i);
                    return execute_compound_list(&elif_body, state);
                }
                // Skip elif body.
                let (_, consumed) = deserialize_list(words, i);
                i += consumed;
            }
            _ => { i += 1; } // skip unknown tag
        }
    }
}

// ---------------------------------------------------------------------------
// §2.9.4.5/.6 While/Until loop
// ---------------------------------------------------------------------------

/// Execute a `while cond; do body; done` or `until cond; do body; done` loop.
///
/// Serialization: TAG_WHILE|TAG_UNTIL <cond-list> TAG_BODY <body-list>
fn execute_while(cmd: &SimpleCommand, _in_fd: i32, _out_fd: i32, state: &mut ShellState, is_until: bool) -> i32 {
    let words = &cmd.words;
    let mut i = 1usize; // skip tag

    // Deserialize condition and body once (static structure).
    let (cond_list, consumed) = deserialize_list(words, i);
    i += consumed;
    if words.get(i).map(|s| s.as_str()) == Some(TAG_BODY) { i += 1; }
    let (body_list, _) = deserialize_list(words, i);

    state.loop_depth += 1;
    let mut last_status = 0i32;

    loop {
        // Evaluate condition.
        let cond_status = execute_compound_list(&cond_list, state);
        // while: continue if cond_status == 0; until: continue if cond_status != 0
        let should_run = if is_until { cond_status != 0 } else { cond_status == 0 };
        if !should_run { break; }

        last_status = execute_compound_list(&body_list, state);

        // Check loop control signal.
        match &state.loop_signal {
            crate::LoopSignal::Break(1) => {
                state.loop_signal = crate::LoopSignal::None;
                break;
            }
            crate::LoopSignal::Break(n) if *n > 1 => {
                let n = n - 1;
                state.loop_signal = crate::LoopSignal::Break(n);
                break;
            }
            crate::LoopSignal::Continue(1) => {
                state.loop_signal = crate::LoopSignal::None;
                // Continue to next iteration.
            }
            crate::LoopSignal::Continue(n) if *n > 1 => {
                let n = n - 1;
                state.loop_signal = crate::LoopSignal::Continue(n);
                break;
            }
            _ => {}
        }
    }

    state.loop_depth -= 1;
    last_status
}

// ---------------------------------------------------------------------------
// §2.9.4.3 Case conditional
// ---------------------------------------------------------------------------

/// Execute a `case word in pattern) body;; ... esac` command.
///
/// Serialization: TAG_CASE <subject> [TAG_CASE_ITEM <N> p1..pN TAG_BODY <body> sep] ... TAG_ESAC
fn execute_case(cmd: &SimpleCommand, _in_fd: i32, _out_fd: i32, state: &mut ShellState) -> i32 {
    let words = &cmd.words;
    let mut i = 1usize; // skip TAG_CASE

    // Expand subject.
    let raw_subject = words.get(i).cloned().unwrap_or_default();
    i += 1;
    let subject = expand_word_nosplit(&raw_subject, state);

    let mut last_status = 0i32;
    let mut matched = false;

    loop {
        match words.get(i).map(|s| s.as_str()) {
            Some(TAG_ESAC) | None => break,
            Some(TAG_CASE_ITEM) => {
                i += 1;
                let pattern_count: usize = words.get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                i += 1;

                let mut patterns: Vec<String> = Vec::new();
                for _ in 0..pattern_count {
                    patterns.push(words.get(i).cloned().unwrap_or_default());
                    i += 1;
                }

                if words.get(i).map(|s| s.as_str()) == Some(TAG_BODY) { i += 1; }

                let (body_list, consumed) = deserialize_list(words, i);
                i += consumed;

                // Separator: ";;" | ";&" | ";;&"
                let sep = words.get(i).cloned().unwrap_or(";;".into());
                if !sep.starts_with('\x00') { i += 1; } // skip non-tag separator string

                if !matched {
                    // Check if any pattern matches the subject.
                    let any_match = patterns.iter().any(|p| {
                        let expanded = expand_word_nosplit(p, state);
                        pattern_matches(&expanded, &subject)
                    });

                    if any_match {
                        matched = true;
                        last_status = execute_compound_list(&body_list, state);

                        match sep.as_str() {
                            ";;" => break, // stop matching
                            ";&" => { /* fall through — run next clause body without testing */ }
                            ";;&" => { matched = false; } // try next pattern
                            _ => break,
                        }
                    }
                } else if sep == ";&" {
                    // Fall-through: execute this clause's body unconditionally.
                    last_status = execute_compound_list(&body_list, state);
                    matched = false; // stop falling through unless next sep is also `;&`
                    match sep.as_str() {
                        ";&" => { matched = true; } // continue fall-through
                        _ => {}
                    }
                }
            }
            _ => { i += 1; } // skip unknown
        }
    }

    last_status
}

// ---------------------------------------------------------------------------
// §2.9.5 Function definition and call
// ---------------------------------------------------------------------------

/// Record a function definition in `state.functions`.
///
/// Serialization: words[0]=TAG_FUNCDEF, words[1]=name, words[2..]=body_words
fn execute_funcdef(cmd: &SimpleCommand, state: &mut ShellState) -> i32 {
    if cmd.words.len() < 3 {
        write_err("sh: invalid function definition\n");
        return 1;
    }
    let name = cmd.words[1].clone();
    let body_words: Vec<String> = cmd.words[2..].to_vec();

    // Replace existing definition if the function was already defined.
    if let Some(entry) = state.functions.iter_mut().find(|(n, _)| n.as_str() == name.as_str()) {
        entry.1 = body_words;
    } else {
        state.functions.push((name, body_words));
    }
    0
}

/// Execute a previously defined shell function.
///
/// §2.9.5: The function body runs in the current environment. Positional
/// parameters are set to the call arguments for the duration of the call.
fn execute_function_call(
    body_words: &[String],
    call_args: &[String],
    redirects: &[Redirect],
    _in_fd: i32,
    _out_fd: i32,
    assignments: &[(String, String)],
    state: &mut ShellState,
) -> i32 {
    // Apply pre-command variable assignments to the current environment.
    for (name, value) in assignments {
        let _ = state.vars.set(name.as_str(), value.as_str());
    }

    // Save and replace positional parameters.
    let saved_positional = core::mem::replace(
        &mut state.positional_params,
        call_args.to_vec(),
    );

    // Save and replace redirections scope (no-op: we don't have fd save/restore yet;
    // functions inherit the caller's I/O per POSIX).
    // Apply any per-call redirects that were on the function call itself.
    let redirect_fds = if !redirects.is_empty() {
        open_redirects(redirects)
    } else {
        Some((None, None))
    };
    let redirect_fds = match redirect_fds {
        Some(fds) => fds,
        None => {
            state.positional_params = saved_positional;
            return 1;
        }
    };

    // Reconstruct a body SimpleCommand and run it.
    let body_cmd = SimpleCommand {
        assignments: Vec::new(),
        words: body_words.to_vec(),
        redirects: Vec::new(),
    };
    // Apply redirects by temporarily patching the command.
    // Since body_words already encodes the body (e.g. TAG_GROUP), apply any
    // call-site redirects through the existing open_redirects mechanism.
    let (redir_in, redir_out) = redirect_fds;
    let effective_in  = redir_in.unwrap_or(0);
    let effective_out = redir_out.unwrap_or(1);

    state.function_depth += 1;
    let status = execute_command(&body_cmd, effective_in, effective_out, state);

    // Consume any pending `return` signal.
    let final_status = if let Some(ret) = state.return_signal.take() {
        ret
    } else {
        status
    };
    state.function_depth -= 1;

    close_optional(redir_in);
    close_optional(redir_out);

    // Restore positional parameters.
    state.positional_params = saved_positional;

    final_status
}

// ---------------------------------------------------------------------------
// §2.6.3 Command substitution
// ---------------------------------------------------------------------------

/// Execute `cmd_text` in a subshell and capture its stdout.
///
/// §2.6.3: The output is subjected to field splitting; trailing newlines are
/// stripped. The substitution runs in a fork with stdout redirected to a pipe.
///
/// This function has the signature required by `ShellState::command_sub_fn`.
pub fn command_substitution(cmd_text: &str, state: &mut ShellState) -> String {
    // Parse the command text.
    let tokens = match crate::lexer::tokenize(cmd_text) {
        Ok(t) => t,
        Err(_) => return String::new(),
    };
    let list = match crate::parser::parse_compound_list(&tokens) {
        Ok(l) => l,
        Err(_) => return String::new(),
    };

    // Create a pipe: child writes to write-end, parent reads from read-end.
    let mut pipe_fds = [0i32; 2];
    if raw::raw_pipe(pipe_fds.as_mut_ptr()) < 0 {
        return String::new();
    }
    let read_fd  = pipe_fds[0];
    let write_fd = pipe_fds[1];

    let pid = raw::raw_fork();
    if pid < 0 {
        raw::raw_close(read_fd);
        raw::raw_close(write_fd);
        return String::new();
    }

    if pid == 0 {
        // Child: dup write-end to stdout, close read-end.
        raw::raw_close(read_fd);
        raw::raw_dup2(write_fd, 1);
        raw::raw_close(write_fd);
        let status = execute_compound_list(&list, state);
        raw::raw_exit(status);
    }

    // Parent: close write-end, read from read-end until EOF.
    raw::raw_close(write_fd);

    let mut output = String::new();
    let mut buf = [0u8; 256];
    loop {
        let n = raw::raw_read(read_fd, buf.as_mut_ptr(), buf.len());
        if n <= 0 { break; }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            output.push_str(s);
        }
    }
    raw::raw_close(read_fd);

    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    let exit_status = decode_wait_status(raw_status);
    state.last_exit_status = exit_status;

    // §2.6.3: strip trailing newlines.
    let trimmed = output.trim_end_matches('\n').trim_end_matches('\r');
    String::from(trimmed)
}

// ---------------------------------------------------------------------------
// §2.9.1.2: Special built-in classification
// ---------------------------------------------------------------------------

fn is_special_builtin(name: &str) -> bool {
    matches!(name,
        ":" | "." | "break" | "continue" | "eval" | "exec" | "exit" |
        "export" | "readonly" | "return" | "set" | "shift" | "times" |
        "trap" | "unset"
    )
}

// ---------------------------------------------------------------------------
// §2.9.1.6: fork + exec for external commands
// ---------------------------------------------------------------------------

/// Fork and exec a non-built-in utility.
///
/// Parent emits 127 before forking if PATH search fails.
/// ENOEXEC heuristic: re-exec via `sh` if the file starts with `#!`.
fn fork_and_exec_expanded(
    cmd: &SimpleCommand,
    in_fd: i32,
    out_fd: i32,
    assignments: &[(String, String)],
    state: &mut ShellState,
) -> i32 {
    if cmd.words.is_empty() { return 0; }

    let path = match resolve_command_with_path(&cmd.words[0], state) {
        Some(p) => p,
        None => {
            write_err("sh: ");
            write_err(cmd.words[0].as_str());
            write_err(": command not found\n");
            return 127;
        }
    };

    let redirect_fds = match open_redirects(&cmd.redirects) {
        Some(fds) => fds,
        None => return 1,
    };
    let (redir_in, redir_out) = redirect_fds;

    let pid = raw::raw_fork();
    if pid < 0 {
        write_err("sh: fork failed\n");
        close_optional(redir_in);
        close_optional(redir_out);
        return 1;
    }

    if pid == 0 {
        for (name, value) in assignments {
            let _ = state.vars.set(name.as_str(), value.as_str());
            state.vars.export(name.as_str());
        }

        apply_io(in_fd, out_fd, redir_in, redir_out);

        let mut fd = 3i32;
        while fd < 64 { raw::raw_close(fd); fd += 1; }

        let mut argv_flat: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        for word in &cmd.words {
            argv_flat.extend_from_slice(word.as_bytes());
            argv_flat.push(0u8);
        }

        // §2.5.3: Build envp from exported variables.
        let (envp_flat, _envp_offsets) = state.vars.build_envp();

        raw::raw_execve(
            path.as_ptr(), path.len(),
            argv_flat.as_ptr(), argv_flat.len(),
            envp_flat.as_ptr(), envp_flat.len(),
        );

        // ENOEXEC: check for shebang.
        let mut hdr = [0u8; 2];
        let hdr_fd = raw::raw_open(path.as_ptr(), path.len());
        let is_script = if hdr_fd >= 0 {
            let n = raw::raw_read(hdr_fd as i32, hdr.as_mut_ptr(), 2);
            raw::raw_close(hdr_fd as i32);
            n == 2 && hdr[0] == b'#' && hdr[1] == b'!'
        } else {
            false
        };

        if is_script {
            let sh_path = b"/system/bin/sh\0";
            let mut new_argv: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            new_argv.extend_from_slice(b"sh\0");
            new_argv.extend_from_slice(path.as_bytes());
            new_argv.push(0u8);
            for word in cmd.words.iter().skip(1) {
                new_argv.extend_from_slice(word.as_bytes());
                new_argv.push(0u8);
            }
            raw::raw_execve(
                sh_path.as_ptr(), sh_path.len() - 1,
                new_argv.as_ptr(), new_argv.len(),
                envp_flat.as_ptr(), envp_flat.len(),
            );
        }

        write_err(cmd.words[0].as_str());
        write_err(": cannot execute\n");
        raw::raw_exit(126);
    }

    close_optional(redir_in);
    close_optional(redir_out);

    raw::raw_setfgpid(pid as i32);
    let mut raw_status = 0i32;
    raw::raw_wait(pid as i32, &mut raw_status);
    raw::raw_setfgpid(0);

    decode_wait_status(raw_status)
}

// ---------------------------------------------------------------------------
// Redirect helpers
// ---------------------------------------------------------------------------

fn expand_redirect(redirect: &Redirect, state: &mut ShellState) -> Redirect {
    match redirect {
        Redirect::StdinFrom(fd, f)          => Redirect::StdinFrom(*fd,  expand_word_nosplit(f, state)),
        Redirect::StdoutTo(fd, f)           => Redirect::StdoutTo(*fd,   expand_word_nosplit(f, state)),
        Redirect::StdoutAppend(fd, f)       => Redirect::StdoutAppend(*fd, expand_word_nosplit(f, state)),
        Redirect::StdoutNoclobber(fd, f)    => Redirect::StdoutNoclobber(*fd, expand_word_nosplit(f, state)),
        Redirect::DupOut(fd, f)             => Redirect::DupOut(*fd,     expand_word_nosplit(f, state)),
        Redirect::DupIn(fd, f)              => Redirect::DupIn(*fd,      expand_word_nosplit(f, state)),
        Redirect::ReadWrite(fd, f)          => Redirect::ReadWrite(*fd,  expand_word_nosplit(f, state)),
        Redirect::HereDoc(fd, strip, delim) => Redirect::HereDoc(*fd, *strip, delim.clone()),
    }
}

fn open_redirects(redirects: &[Redirect]) -> Option<(Option<i32>, Option<i32>)> {
    let mut redir_in:  Option<i32> = None;
    let mut redir_out: Option<i32> = None;

    for redirect in redirects {
        match redirect {
            Redirect::StdinFrom(_fd, path) => {
                let fd = raw::raw_open(path.as_ptr(), path.len());
                if fd < 0 {
                    write_err("sh: ");
                    write_err(path.as_str());
                    write_err(": no such file or directory\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
                close_optional(redir_in);
                redir_in = Some(fd as i32);
            }

            Redirect::StdoutTo(_fd, path) | Redirect::StdoutNoclobber(_fd, path) => {
                let fd = raw::raw_creat(path.as_ptr(), path.len());
                if fd < 0 {
                    write_err("sh: ");
                    write_err(path.as_str());
                    write_err(": cannot create file\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
                close_optional(redir_out);
                redir_out = Some(fd as i32);
            }

            Redirect::StdoutAppend(_fd, path) => {
                let fd = raw::raw_creat_append(path.as_ptr(), path.len());
                if fd < 0 {
                    write_err("sh: ");
                    write_err(path.as_str());
                    write_err(": cannot open file\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
                raw::raw_seek(fd as i32, 0, 2);
                close_optional(redir_out);
                redir_out = Some(fd as i32);
            }

            Redirect::ReadWrite(_fd, path) => {
                let fd = raw::raw_creat_append(path.as_ptr(), path.len());
                if fd < 0 {
                    write_err("sh: ");
                    write_err(path.as_str());
                    write_err(": cannot open file for read/write\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
                close_optional(redir_in);
                redir_in = Some(fd as i32);
            }

            Redirect::DupIn(explicit_fd, word) => {
                let _target_fd = explicit_fd.unwrap_or(0) as i32;
                if word == "-" {
                    if explicit_fd.unwrap_or(0) == 0 { close_optional(redir_in); redir_in = None; }
                } else if let Some(src_fd) = parse_fd_word(word) {
                    let new_fd = raw::raw_dup(src_fd) as i32;
                    if new_fd < 0 {
                        write_err("sh: <&: bad file descriptor\n");
                        close_optional(redir_in);
                        close_optional(redir_out);
                        return None;
                    }
                    close_optional(redir_in);
                    redir_in = Some(new_fd);
                } else {
                    write_err("sh: <&: ");
                    write_err(word.as_str());
                    write_err(": not a valid file descriptor\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
            }

            Redirect::DupOut(explicit_fd, word) => {
                let _target_fd = explicit_fd.unwrap_or(1) as i32;
                if word == "-" {
                    if explicit_fd.unwrap_or(1) == 1 { close_optional(redir_out); redir_out = None; }
                } else if let Some(src_fd) = parse_fd_word(word) {
                    let new_fd = raw::raw_dup(src_fd) as i32;
                    if new_fd < 0 {
                        write_err("sh: >&: bad file descriptor\n");
                        close_optional(redir_in);
                        close_optional(redir_out);
                        return None;
                    }
                    close_optional(redir_out);
                    redir_out = Some(new_fd);
                } else {
                    write_err("sh: >&: ");
                    write_err(word.as_str());
                    write_err(": not a valid file descriptor\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
            }

            Redirect::HereDoc(_fd, _strip, body) => {
                let mut pipe_fds = [0i32; 2];
                if raw::raw_pipe(pipe_fds.as_mut_ptr()) < 0 {
                    write_err("sh: heredoc: pipe failed\n");
                    close_optional(redir_in);
                    close_optional(redir_out);
                    return None;
                }
                raw::raw_write(pipe_fds[1], body.as_ptr(), body.len());
                raw::raw_close(pipe_fds[1]);
                close_optional(redir_in);
                redir_in = Some(pipe_fds[0]);
            }
        }
    }

    Some((redir_in, redir_out))
}

fn parse_fd_word(word: &str) -> Option<i32> {
    let mut result: i32 = 0;
    for c in word.chars() {
        let d = c.to_digit(10)? as i32;
        result = result.checked_mul(10)?.checked_add(d)?;
    }
    Some(result)
}

fn apply_io(in_fd: i32, out_fd: i32, redir_in: Option<i32>, redir_out: Option<i32>) {
    if in_fd != 0 { raw::raw_dup2(in_fd, 0); raw::raw_close(in_fd); }
    if out_fd != 1 { raw::raw_dup2(out_fd, 1); raw::raw_close(out_fd); }
    if let Some(fd) = redir_in  { raw::raw_dup2(fd, 0); raw::raw_close(fd); }
    if let Some(fd) = redir_out { raw::raw_dup2(fd, 1); raw::raw_close(fd); }
}

/// §2.8.2: The kernel stores the child's raw exit code directly.
#[inline]
fn decode_wait_status(raw_status: i32) -> i32 {
    ((raw_status as u32) & 0xFF) as i32
}

#[inline]
fn close_optional(fd: Option<i32>) {
    if let Some(f) = fd { raw::raw_close(f); }
}

// ---------------------------------------------------------------------------
// PATH resolution
// ---------------------------------------------------------------------------

fn resolve_command_with_path(name: &str, state: &ShellState) -> Option<String> {
    if name.contains('/') { return Some(name.into()); }

    let path_var = state.vars.get("PATH").unwrap_or("/system/bin");
    for dir in path_var.split(':') {
        if dir.is_empty() { continue; }
        let mut candidate = String::from(dir);
        if !candidate.ends_with('/') { candidate.push('/'); }
        candidate.push_str(name);
        let fd = raw::raw_open(candidate.as_ptr(), candidate.len());
        if fd >= 0 {
            raw::raw_close(fd as i32);
            return Some(candidate);
        }
    }

    None
}
