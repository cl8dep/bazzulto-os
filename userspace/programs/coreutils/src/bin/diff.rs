// POSIX.1-2024 — diff
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/diff.html
//
// Compare two files and write a list of changes to standard output.
// No output is produced if the files are identical.
//
// Supported options:
//   -b        Ignore changes in the amount of white space.
//   -c        Context diff (3 lines of context).
//   -C n      Context diff with n lines of context.
//   -e        Output as an ed(1) script.
//   -f        Output as a reverse ed(1) script.
//   -u        Unified diff (3 lines of context).
//   -U n      Unified diff with n lines of context.
//   -r        Recursive directory comparison via getdents64.
//
// Exit status:
//   0   No differences found.
//   1   Differences found and output successfully.
//   >1  An error occurred.
//
// Algorithm: Myers O(ND) shortest edit script.
// Reference: E.W. Myers, "An O(ND) Difference Algorithm and Its Variations",
//            Algorithmica 1(2), 1986, pp. 251–266.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use bazzulto_system::raw;
use bazzulto_system::time::{DateTime, SystemTime};
use bazzulto_io::directory::read_dir;
use coreutils::{args, open_file, read_fd_to_end, read_stdin_to_end, write_stdout, write_stderr};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum OutputFormat {
    Default,
    EdScript,
    ReverseEdScript,
    Context,
    Unified,
}

struct Options {
    format:                OutputFormat,
    context_lines:         usize,
    ignore_whitespace:     bool,
    recursive:             bool,
}

/// A contiguous block of differing lines between the two files.
/// All line numbers are 1-based and inclusive.
/// old_end < old_start means a pure insertion (no lines removed from old).
/// new_end < new_start means a pure deletion (no lines added from new).
struct Hunk {
    old_start: usize,
    old_end:   usize,
    new_start: usize,
    new_end:   usize,
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_options(arguments: &[String]) -> (Options, &str, &str) {
    let mut format             = OutputFormat::Default;
    let mut context_lines      = 3usize;
    let mut ignore_whitespace  = false;
    let mut recursive          = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut end_of_options     = false;
    let mut index              = 0usize;

    while index < arguments.len() {
        let argument = arguments[index].as_str();
        if end_of_options || !argument.starts_with('-') || argument == "-" {
            positional.push(argument);
            index += 1;
            continue;
        }
        if argument == "--" {
            end_of_options = true;
            index += 1;
            continue;
        }
        // Strip the leading '-' and process each character.
        let chars: Vec<char> = argument[1..].chars().collect();
        let mut char_index = 0usize;
        while char_index < chars.len() {
            match chars[char_index] {
                'b' => ignore_whitespace = true,
                'r' => recursive = true,
                'c' => { format = OutputFormat::Context; context_lines = 3; }
                'u' => { format = OutputFormat::Unified; context_lines = 3; }
                'e' => format = OutputFormat::EdScript,
                'f' => format = OutputFormat::ReverseEdScript,
                'r' => {
                    write_stderr("diff: -r (recursive) is not supported\n");
                    raw::raw_exit(2);
                }
                'C' | 'U' => {
                    let requested_format = if chars[char_index] == 'C' {
                        OutputFormat::Context
                    } else {
                        OutputFormat::Unified
                    };
                    format = requested_format;
                    // The number may be attached (e.g. -C5) or separate (e.g. -C 5).
                    let rest: String = chars[char_index + 1..].iter().collect();
                    if !rest.is_empty() {
                        context_lines = parse_usize_or_die(&rest, chars[char_index]);
                        char_index = chars.len(); // consumed rest of this argument
                    } else {
                        index += 1;
                        if index >= arguments.len() {
                            write_stderr("diff: option requires an argument\n");
                            raw::raw_exit(2);
                        }
                        context_lines = parse_usize_or_die(arguments[index].as_str(), chars[char_index]);
                    }
                }
                other => {
                    write_stderr("diff: invalid option -- '");
                    let mut byte_buf = [0u8; 4];
                    write_stderr(other.encode_utf8(&mut byte_buf));
                    write_stderr("'\n");
                    raw::raw_exit(2);
                }
            }
            char_index += 1;
        }
        index += 1;
    }

    if positional.len() != 2 {
        write_stderr("diff: requires exactly two file operands\n");
        raw::raw_exit(2);
    }

    (
        Options { format, context_lines, ignore_whitespace, recursive },
        positional[0],
        positional[1],
    )
}

fn parse_usize_or_die(source: &str, option_char: char) -> usize {
    let mut result = 0usize;
    let mut has_digit = false;
    for character in source.chars() {
        if let Some(digit) = character.to_digit(10) {
            result = result.saturating_mul(10).saturating_add(digit as usize);
            has_digit = true;
        } else {
            break;
        }
    }
    if !has_digit {
        write_stderr("diff: option -");
        let mut byte_buf = [0u8; 4];
        write_stderr(option_char.encode_utf8(&mut byte_buf));
        write_stderr(" requires a numeric argument\n");
        raw::raw_exit(2);
    }
    result
}

// ---------------------------------------------------------------------------
// Myers O(ND) diff — returns list of edits as (old_index, new_index) pairs
// ---------------------------------------------------------------------------

/// Compute the shortest edit script between `old_lines` and `new_lines`.
/// Returns a Vec<(old_index_opt, new_index_opt)> where:
///   (Some(i), None)       = delete old_lines[i]
///   (None,    Some(j))    = insert new_lines[j]
///   (Some(i), Some(j))    = equal (old_lines[i] == new_lines[j])
fn myers_diff<'a>(
    old_lines: &'a [&'a str],
    new_lines: &'a [&'a str],
    ignore_whitespace: bool,
) -> Vec<(Option<usize>, Option<usize>)> {
    let old_len = old_lines.len();
    let new_len = new_lines.len();

    // Trim common prefix.
    let mut prefix_len = 0usize;
    while prefix_len < old_len
        && prefix_len < new_len
        && lines_equal(old_lines[prefix_len], new_lines[prefix_len], ignore_whitespace)
    {
        prefix_len += 1;
    }

    // Trim common suffix.
    let mut suffix_len = 0usize;
    while suffix_len < old_len - prefix_len
        && suffix_len < new_len - prefix_len
        && lines_equal(
            old_lines[old_len - 1 - suffix_len],
            new_lines[new_len - 1 - suffix_len],
            ignore_whitespace,
        )
    {
        suffix_len += 1;
    }

    let old_trimmed = &old_lines[prefix_len..old_len - suffix_len];
    let new_trimmed = &new_lines[prefix_len..new_len - suffix_len];

    let mut result: Vec<(Option<usize>, Option<usize>)> = Vec::new();

    // Emit common prefix as equal pairs.
    for i in 0..prefix_len {
        result.push((Some(i), Some(i)));
    }

    if old_trimmed.is_empty() && new_trimmed.is_empty() {
        // Emit common suffix.
        for k in 0..suffix_len {
            result.push((Some(prefix_len + k), Some(prefix_len + k)));
        }
        return result;
    }

    // Run Myers on the trimmed middle.
    let middle_edits = myers_core(old_trimmed, new_trimmed, ignore_whitespace, prefix_len);
    result.extend(middle_edits);

    // Emit common suffix.
    for k in 0..suffix_len {
        let old_idx = old_len - suffix_len + k;
        let new_idx = new_len - suffix_len + k;
        result.push((Some(old_idx), Some(new_idx)));
    }

    result
}

fn myers_core(
    old_lines: &[&str],
    new_lines: &[&str],
    ignore_whitespace: bool,
    old_offset: usize,
) -> Vec<(Option<usize>, Option<usize>)> {
    let old_len = old_lines.len();
    let new_len = new_lines.len();
    let max_d   = old_len + new_len;

    if max_d == 0 {
        return Vec::new();
    }

    // Guard against pathological inputs that would exhaust memory.
    // If files are very large and very different, output a simplified diff.
    if max_d > 40_000 {
        let mut result = Vec::new();
        for (i, _) in old_lines.iter().enumerate() {
            result.push((Some(old_offset + i), None));
        }
        for (j, _) in new_lines.iter().enumerate() {
            result.push((None, Some(j)));
        }
        return result;
    }

    // v[k + max_d] = furthest x reached on diagonal k.
    let vector_size = 2 * max_d + 2;
    let mut forward_vector: Vec<i64> = vec![0i64; vector_size];
    // Store snapshots of forward_vector after each d step for backtracking.
    let mut history: Vec<Vec<i64>> = Vec::new();

    let mut found_d = max_d + 1;
    let mut found_k = 0i64;

    'search: for d in 0..=max_d {
        for k_raw in (-(d as i64)..=(d as i64)).step_by(2) {
            let k = k_raw;
            let ki = (k + max_d as i64) as usize;

            let mut x = if k == -(d as i64) || (k != d as i64 && forward_vector[ki - 1] < forward_vector[ki + 1]) {
                forward_vector[ki + 1] as usize
            } else {
                forward_vector[ki - 1] as usize + 1
            };

            let mut y = (x as i64 - k) as usize;

            // Extend along the snake (equal lines).
            while x < old_len
                && y < new_len
                && lines_equal(old_lines[x], new_lines[y], ignore_whitespace)
            {
                x += 1;
                y += 1;
            }

            forward_vector[ki] = x as i64;

            if x >= old_len && y >= new_len {
                found_d = d;
                found_k = k;
                history.push(forward_vector.clone());
                break 'search;
            }
        }
        history.push(forward_vector.clone());
    }

    // Backtrack through history to reconstruct the edit sequence.
    let mut edits: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let mut x = old_len;
    let mut y = new_len;
    let mut d = found_d;
    let mut k = found_k;

    while d > 0 {
        let previous_snapshot = &history[d - 1];
        let ki = (k + max_d as i64) as usize;

        let previous_k_is_above = k == -(d as i64)
            || (k != d as i64 && previous_snapshot[ki - 1] < previous_snapshot[ki + 1]);

        let previous_k = if previous_k_is_above { k + 1 } else { k - 1 };
        let previous_ki = (previous_k + max_d as i64) as usize;
        let previous_x = previous_snapshot[previous_ki] as usize;
        let previous_y = (previous_x as i64 - previous_k) as usize;

        // Walk back the snake (equal lines, in reverse).
        while x > previous_x + (if previous_k_is_above { 0 } else { 1 })
            && y > previous_y + (if previous_k_is_above { 1 } else { 0 })
        {
            x -= 1;
            y -= 1;
            edits.push((Some(old_offset + x), Some(x - old_offset + y - (y - previous_y - (if previous_k_is_above { 1 } else { 0})))));
        }

        if previous_k_is_above {
            // Vertical move: insert from new.
            if y > 0 {
                y -= 1;
                edits.push((None, Some(y)));
            }
        } else {
            // Horizontal move: delete from old.
            if x > 0 {
                x -= 1;
                edits.push((Some(old_offset + x), None));
            }
        }

        k = previous_k;
        d -= 1;
    }

    // Walk back the remaining snake at d=0.
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        edits.push((Some(old_offset + x), Some(old_offset + x)));
    }

    edits.reverse();
    edits
}

// ---------------------------------------------------------------------------
// Hunk grouping
// ---------------------------------------------------------------------------

/// Group the flat edit list into Hunks (contiguous changed regions).
/// Context-aware: hunks that overlap when context_lines are added get merged.
fn group_into_hunks(
    edits: &[(Option<usize>, Option<usize>)],
    context_lines: usize,
) -> Vec<Hunk> {
    // Collect the changed line pairs, tracking positions in each file.
    let mut changed_ranges: Vec<(usize, usize, usize, usize)> = Vec::new(); // (old_start, old_end, new_start, new_end)

    let mut old_pos = 1usize;
    let mut new_pos = 1usize;
    let mut current_old_start = 0usize;
    let mut current_new_start = 0usize;
    let mut in_change = false;

    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(_), Some(_)) => {
                // Equal line.
                if in_change {
                    changed_ranges.push((current_old_start, old_pos - 1, current_new_start, new_pos - 1));
                    in_change = false;
                }
                old_pos += 1;
                new_pos += 1;
            }
            (Some(_), None) => {
                // Deletion.
                if !in_change {
                    current_old_start = old_pos;
                    current_new_start = new_pos;
                    in_change = true;
                }
                old_pos += 1;
            }
            (None, Some(_)) => {
                // Insertion.
                if !in_change {
                    current_old_start = old_pos;
                    current_new_start = new_pos;
                    in_change = true;
                }
                new_pos += 1;
            }
            (None, None) => {}
        }
    }
    if in_change {
        changed_ranges.push((current_old_start, old_pos - 1, current_new_start, new_pos - 1));
    }

    if changed_ranges.is_empty() {
        return Vec::new();
    }

    // Build hunks with context, merging overlapping ones.
    let mut hunks: Vec<Hunk> = Vec::new();

    let first = &changed_ranges[0];
    let mut current_hunk = Hunk {
        old_start: first.0.saturating_sub(context_lines).max(1),
        old_end:   first.1 + context_lines,
        new_start: first.2.saturating_sub(context_lines).max(1),
        new_end:   first.3 + context_lines,
    };

    for range in &changed_ranges[1..] {
        let next_old_start = range.0.saturating_sub(context_lines).max(1);
        if next_old_start <= current_hunk.old_end + 1 {
            // Overlaps — merge.
            current_hunk.old_end = current_hunk.old_end.max(range.1 + context_lines);
            current_hunk.new_end = current_hunk.new_end.max(range.3 + context_lines);
        } else {
            hunks.push(current_hunk);
            current_hunk = Hunk {
                old_start: next_old_start,
                old_end:   range.1 + context_lines,
                new_start: range.2.saturating_sub(context_lines).max(1),
                new_end:   range.3 + context_lines,
            };
        }
    }
    hunks.push(current_hunk);

    hunks
}

// ---------------------------------------------------------------------------
// Output: default ed-style
// ---------------------------------------------------------------------------

fn output_default(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
) {
    // Group into basic change blocks (no context).
    let mut old_pos = 1usize;
    let mut new_pos = 1usize;
    let mut block_old_start = 0usize;
    let mut block_new_start = 0usize;
    let mut block_old_lines: Vec<&str> = Vec::new();
    let mut block_new_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    let flush_block = |old_start: usize,
                       old_end: usize,
                       new_start: usize,
                       new_end: usize,
                       old_block: &[&str],
                       new_block: &[&str]| {
        // Range header: format is "n1[,n2]a|d|c n3[,n4]"
        if old_block.is_empty() {
            // Pure insertion.
            write_num(old_start.saturating_sub(1));
            write_stdout("a");
            write_range(new_start, new_end);
        } else if new_block.is_empty() {
            // Pure deletion.
            write_range(old_start, old_end);
            write_stdout("d");
            write_num(new_start.saturating_sub(1));
        } else {
            // Change.
            write_range(old_start, old_end);
            write_stdout("c");
            write_range(new_start, new_end);
        }
        write_stdout("\n");

        for line in old_block {
            write_stdout("< ");
            write_stdout(line);
            if !line.ends_with('\n') { write_stdout("\n"); }
        }
        if !old_block.is_empty() && !new_block.is_empty() {
            write_stdout("---\n");
        }
        for line in new_block {
            write_stdout("> ");
            write_stdout(line);
            if !line.ends_with('\n') { write_stdout("\n"); }
        }
    };

    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(oi), Some(_)) => {
                if in_block {
                    flush_block(
                        block_old_start, old_pos - 1,
                        block_new_start, new_pos - 1,
                        &block_old_lines, &block_new_lines,
                    );
                    block_old_lines.clear();
                    block_new_lines.clear();
                    in_block = false;
                }
                old_pos += 1;
                new_pos += 1;
                let _ = oi;
            }
            (Some(oi), None) => {
                if !in_block {
                    block_old_start = old_pos;
                    block_new_start = new_pos;
                    in_block = true;
                }
                block_old_lines.push(old_lines[oi]);
                old_pos += 1;
            }
            (None, Some(ni)) => {
                if !in_block {
                    block_old_start = old_pos;
                    block_new_start = new_pos;
                    in_block = true;
                }
                block_new_lines.push(new_lines[ni]);
                new_pos += 1;
            }
            (None, None) => {}
        }
    }
    if in_block {
        flush_block(
            block_old_start, old_pos - 1,
            block_new_start, new_pos - 1,
            &block_old_lines, &block_new_lines,
        );
    }
}

// ---------------------------------------------------------------------------
// Output: -e ed script (hunks in reverse order)
// ---------------------------------------------------------------------------

fn output_ed_script(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
) {
    // Collect change blocks then emit in reverse order.
    let mut blocks: Vec<(usize, usize, usize, usize, Vec<&str>, Vec<&str>)> = Vec::new();
    let mut old_pos = 1usize;
    let mut new_pos = 1usize;
    let mut block_old_start = 0usize;
    let mut block_new_start = 0usize;
    let mut block_old_lines: Vec<&str> = Vec::new();
    let mut block_new_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(_), Some(_)) => {
                if in_block {
                    blocks.push((block_old_start, old_pos - 1, block_new_start, new_pos - 1,
                                 block_old_lines.clone(), block_new_lines.clone()));
                    block_old_lines.clear();
                    block_new_lines.clear();
                    in_block = false;
                }
                old_pos += 1;
                new_pos += 1;
            }
            (Some(oi), None) => {
                if !in_block {
                    block_old_start = old_pos;
                    block_new_start = new_pos;
                    in_block = true;
                }
                block_old_lines.push(old_lines[oi]);
                old_pos += 1;
            }
            (None, Some(ni)) => {
                if !in_block {
                    block_old_start = old_pos;
                    block_new_start = new_pos;
                    in_block = true;
                }
                block_new_lines.push(new_lines[ni]);
                new_pos += 1;
            }
            (None, None) => {}
        }
    }
    if in_block {
        blocks.push((block_old_start, old_pos - 1, block_new_start, new_pos - 1,
                     block_old_lines.clone(), block_new_lines.clone()));
    }

    // Emit in reverse order (so applying the script doesn't shift line numbers).
    for (old_start, old_end, new_start, _new_end, old_block, new_block) in blocks.iter().rev() {
        if old_block.is_empty() {
            // Append after old_start - 1.
            write_num(old_start.saturating_sub(1));
            write_stdout("a\n");
        } else if new_block.is_empty() {
            write_range(*old_start, *old_end);
            write_stdout("d\n");
            continue; // no text body
        } else {
            write_range(*old_start, *old_end);
            write_stdout("c\n");
        }
        for line in new_block {
            // Protect lone '.' lines per POSIX rationale.
            if *line == "." || line.starts_with('.') {
                write_stdout("s/^\\./../\n");
            }
            write_stdout(line);
            if !line.ends_with('\n') { write_stdout("\n"); }
        }
        write_stdout(".\n");
        let _ = new_start;
    }
}

// ---------------------------------------------------------------------------
// Output: -f reverse ed script (forward order)
// ---------------------------------------------------------------------------

fn output_reverse_ed_script(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
) {
    let mut old_pos = 1usize;
    let mut new_pos = 1usize;
    let mut block_old_start = 0usize;
    let mut block_new_start = 0usize;
    let mut block_old_lines: Vec<&str> = Vec::new();
    let mut block_new_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    let emit_block = |old_start: usize,
                      old_end: usize,
                      new_start: usize,
                      new_end: usize,
                      old_block: &[&str],
                      new_block: &[&str]| {
        // -f: command letter comes first, ranges use space separator.
        if old_block.is_empty() {
            write_stdout("a");
            write_num(new_start);
            if new_end > new_start { write_stdout(" "); write_num(new_end); }
        } else if new_block.is_empty() {
            write_stdout("d");
            write_num(old_start);
            if old_end > old_start { write_stdout(" "); write_num(old_end); }
        } else {
            write_stdout("c");
            write_num(old_start);
            if old_end > old_start { write_stdout(" "); write_num(old_end); }
        }
        write_stdout("\n");
        for line in new_block {
            write_stdout(line);
            if !line.ends_with('\n') { write_stdout("\n"); }
        }
        if !new_block.is_empty() {
            write_stdout(".\n");
        }
        let _ = (new_end, old_end);
    };

    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(_), Some(_)) => {
                if in_block {
                    emit_block(block_old_start, old_pos - 1, block_new_start, new_pos - 1,
                               &block_old_lines, &block_new_lines);
                    block_old_lines.clear();
                    block_new_lines.clear();
                    in_block = false;
                }
                old_pos += 1;
                new_pos += 1;
            }
            (Some(oi), None) => {
                if !in_block { block_old_start = old_pos; block_new_start = new_pos; in_block = true; }
                block_old_lines.push(old_lines[oi]);
                old_pos += 1;
            }
            (None, Some(ni)) => {
                if !in_block { block_old_start = old_pos; block_new_start = new_pos; in_block = true; }
                block_new_lines.push(new_lines[ni]);
                new_pos += 1;
            }
            (None, None) => {}
        }
    }
    if in_block {
        emit_block(block_old_start, old_pos - 1, block_new_start, new_pos - 1,
                   &block_old_lines, &block_new_lines);
    }
}

// ---------------------------------------------------------------------------
// Output: -c / -C context format
// ---------------------------------------------------------------------------

fn output_context(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
    old_name: &str,
    new_name: &str,
    context_lines: usize,
) {
    let hunks = group_into_hunks(edits, context_lines);
    if hunks.is_empty() { return; }

    let old_timestamp = current_timestamp();
    let new_timestamp = current_timestamp();

    write_stdout("*** ");
    write_stdout(old_name);
    write_stdout("\t");
    write_stdout(&old_timestamp);
    write_stdout("\n--- ");
    write_stdout(new_name);
    write_stdout("\t");
    write_stdout(&new_timestamp);
    write_stdout("\n");

    // Build edit classification per-line from the flat edit list.
    // old_class[i] = true if old line i was changed/deleted.
    // new_class[j] = true if new line j was changed/inserted.
    let mut old_changed: Vec<bool> = vec![false; old_lines.len()];
    let mut new_changed: Vec<bool> = vec![false; new_lines.len()];

    let mut old_pos_track = 0usize;
    let mut new_pos_track = 0usize;
    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(oi), Some(_)) => { old_pos_track = oi + 1; new_pos_track += 1; }
            (Some(oi), None)    => { old_changed[oi] = true; old_pos_track = oi + 1; }
            (None, Some(ni))    => { new_changed[ni] = true; new_pos_track = ni + 1; }
            (None, None)        => {}
        }
        let _ = (old_pos_track, new_pos_track);
    }

    for hunk in &hunks {
        write_stdout("***************\n");

        // Old-side header.
        let old_hunk_start = hunk.old_start;
        let old_hunk_end   = hunk.old_end.min(old_lines.len());
        if old_hunk_start == old_hunk_end {
            write_stdout("*** ");
            write_num(old_hunk_end);
            write_stdout(" ****\n");
        } else {
            write_stdout("*** ");
            write_num(old_hunk_start);
            write_stdout(",");
            write_num(old_hunk_end);
            write_stdout(" ****\n");
        }

        // Old-side lines — only emit if there are any changes in this hunk.
        let old_has_changes = (old_hunk_start..=old_hunk_end)
            .any(|n| n >= 1 && n <= old_lines.len() && old_changed[n - 1]);
        if old_has_changes {
            for line_number in old_hunk_start..=old_hunk_end {
                if line_number < 1 || line_number > old_lines.len() { continue; }
                let line = old_lines[line_number - 1];
                if old_changed[line_number - 1] {
                    write_stdout("- ");
                } else {
                    write_stdout("  ");
                }
                write_stdout(line);
                if !line.ends_with('\n') { write_stdout("\n"); }
            }
        }

        // New-side header.
        let new_hunk_start = hunk.new_start;
        let new_hunk_end   = hunk.new_end.min(new_lines.len());
        if new_hunk_start == new_hunk_end {
            write_stdout("--- ");
            write_num(new_hunk_end);
            write_stdout(" ----\n");
        } else {
            write_stdout("--- ");
            write_num(new_hunk_start);
            write_stdout(",");
            write_num(new_hunk_end);
            write_stdout(" ----\n");
        }

        // New-side lines — only emit if there are any changes in this hunk.
        let new_has_changes = (new_hunk_start..=new_hunk_end)
            .any(|n| n >= 1 && n <= new_lines.len() && new_changed[n - 1]);
        if new_has_changes {
            for line_number in new_hunk_start..=new_hunk_end {
                if line_number < 1 || line_number > new_lines.len() { continue; }
                let line = new_lines[line_number - 1];
                if new_changed[line_number - 1] {
                    write_stdout("+ ");
                } else {
                    write_stdout("  ");
                }
                write_stdout(line);
                if !line.ends_with('\n') { write_stdout("\n"); }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Output: -u / -U unified format
// ---------------------------------------------------------------------------

fn output_unified(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
    old_name: &str,
    new_name: &str,
    context_lines: usize,
) {
    let hunks = group_into_hunks(edits, context_lines);
    if hunks.is_empty() { return; }

    let old_timestamp = current_timestamp();
    let new_timestamp = current_timestamp();

    write_stdout("--- ");
    write_stdout(old_name);
    write_stdout("\t");
    write_stdout(&old_timestamp);
    write_stdout("\n+++ ");
    write_stdout(new_name);
    write_stdout("\t");
    write_stdout(&new_timestamp);
    write_stdout("\n");

    // Build change classification per line.
    let mut old_changed: Vec<bool> = vec![false; old_lines.len()];
    let mut new_changed: Vec<bool> = vec![false; new_lines.len()];
    for &(old_opt, new_opt) in edits {
        match (old_opt, new_opt) {
            (Some(oi), None) => old_changed[oi] = true,
            (None, Some(ni)) => new_changed[ni] = true,
            _ => {}
        }
    }

    // For each hunk, walk both sides simultaneously using the edit list.
    // We rebuild a per-hunk edit list from the global one for simplicity.
    for hunk in &hunks {
        let old_start = hunk.old_start;
        let old_end   = hunk.old_end.min(old_lines.len());
        let new_start = hunk.new_start;
        let new_end   = hunk.new_end.min(new_lines.len());

        let old_count = if old_end >= old_start { old_end - old_start + 1 } else { 0 };
        let new_count = if new_end >= new_start { new_end - new_start + 1 } else { 0 };

        // @@ header: POSIX uses %1d (no zero-padding, allows leading spaces).
        write_stdout("@@ -");
        write_num(old_start);
        write_stdout(",");
        write_num(old_count);
        write_stdout(" +");
        write_num(new_start);
        write_stdout(",");
        write_num(new_count);
        write_stdout(" @@\n");

        // Emit unified hunk body.
        // Strategy: walk old_start..old_end emitting context/deleted lines,
        // interleaving inserted new lines when old_changed transitions.
        let mut new_cursor = new_start;

        for old_line_number in old_start..=old_end {
            if old_line_number < 1 || old_line_number > old_lines.len() { break; }
            let old_line = old_lines[old_line_number - 1];

            if old_changed[old_line_number - 1] {
                // Emit any new lines that precede this deletion.
                while new_cursor <= new_end
                    && new_cursor >= 1
                    && new_cursor <= new_lines.len()
                    && new_changed[new_cursor - 1]
                {
                    write_stdout("+");
                    write_stdout(new_lines[new_cursor - 1]);
                    if !new_lines[new_cursor - 1].ends_with('\n') { write_stdout("\n"); }
                    new_cursor += 1;
                }
                write_stdout("-");
                write_stdout(old_line);
                if !old_line.ends_with('\n') { write_stdout("\n"); }
            } else {
                // Context line — advance new_cursor past any pending insertions.
                while new_cursor <= new_end
                    && new_cursor >= 1
                    && new_cursor <= new_lines.len()
                    && new_changed[new_cursor - 1]
                {
                    write_stdout("+");
                    write_stdout(new_lines[new_cursor - 1]);
                    if !new_lines[new_cursor - 1].ends_with('\n') { write_stdout("\n"); }
                    new_cursor += 1;
                }
                write_stdout(" ");
                write_stdout(old_line);
                if !old_line.ends_with('\n') { write_stdout("\n"); }
                new_cursor += 1;
            }
        }

        // Emit any trailing insertions.
        while new_cursor <= new_end
            && new_cursor >= 1
            && new_cursor <= new_lines.len()
        {
            if new_changed[new_cursor - 1] {
                write_stdout("+");
                write_stdout(new_lines[new_cursor - 1]);
                if !new_lines[new_cursor - 1].ends_with('\n') { write_stdout("\n"); }
            }
            new_cursor += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Whitespace comparison helper
// ---------------------------------------------------------------------------

fn lines_equal(a: &str, b: &str, ignore_whitespace: bool) -> bool {
    if !ignore_whitespace {
        return a == b;
    }
    // POSIX -b: trailing whitespace ignored; internal whitespace runs collapsed.
    let a_trimmed = a.trim_end();
    let b_trimmed = b.trim_end();
    let mut a_words = a_trimmed.split_whitespace();
    let mut b_words = b_trimmed.split_whitespace();
    loop {
        match (a_words.next(), b_words.next()) {
            (None, None)                => return true,
            (Some(x), Some(y)) if x == y => continue,
            _                           => return false,
        }
    }
}

// ---------------------------------------------------------------------------
// Timestamp formatting for -c/-u headers
// ---------------------------------------------------------------------------

fn current_timestamp() -> String {
    let system_time = SystemTime::now();
    let unix_seconds = system_time.unix_timestamp();
    let datetime = DateTime::from_unix(unix_seconds, 0);

    let mut output = String::new();
    push_padded_u32(&mut output, datetime.year as u32, 4);
    output.push('-');
    push_padded_u32(&mut output, datetime.month as u32, 2);
    output.push('-');
    push_padded_u32(&mut output, datetime.day as u32, 2);
    output.push(' ');
    push_padded_u32(&mut output, datetime.hour as u32, 2);
    output.push(':');
    push_padded_u32(&mut output, datetime.minute as u32, 2);
    output.push(':');
    push_padded_u32(&mut output, datetime.second as u32, 2);
    output.push_str(".000000000 +0000");
    output
}

fn push_padded_u32(target: &mut String, value: u32, width: usize) {
    let mut buffer = [b'0'; 10];
    let mut pos = 10usize;
    let mut remaining = value;
    if remaining == 0 {
        pos -= 1;
        buffer[pos] = b'0';
    } else {
        while remaining > 0 {
            pos -= 1;
            buffer[pos] = b'0' + (remaining % 10) as u8;
            remaining /= 10;
        }
    }
    let digit_count = 10 - pos;
    for _ in digit_count..width {
        target.push('0');
    }
    for &byte in &buffer[pos..] {
        target.push(byte as char);
    }
}

// ---------------------------------------------------------------------------
// Number formatting helpers
// ---------------------------------------------------------------------------

/// Write a usize as decimal to stdout.
fn write_num(value: usize) {
    let mut buffer = [0u8; 24];
    let mut pos = 24usize;
    let mut remaining = value;
    if remaining == 0 {
        pos -= 1;
        buffer[pos] = b'0';
    } else {
        while remaining > 0 {
            pos -= 1;
            buffer[pos] = b'0' + (remaining % 10) as u8;
            remaining /= 10;
        }
    }
    let slice = unsafe { core::str::from_utf8_unchecked(&buffer[pos..]) };
    write_stdout(slice);
}

/// Write a range "n" or "n1,n2".
fn write_range(start: usize, end: usize) {
    write_num(start);
    if end > start {
        write_stdout(",");
        write_num(end);
    }
}

// ---------------------------------------------------------------------------
// Recursive directory diff (-r)
// ---------------------------------------------------------------------------

/// Compare two directories recursively.
/// Returns true if any differences were found.
fn diff_directories(
    old_dir: &str,
    new_dir: &str,
    options: &Options,
    had_diff: &mut bool,
) {
    // Ensure paths end with '/' for concatenation.
    let old_prefix: String = if old_dir.ends_with('/') {
        String::from(old_dir)
    } else {
        let mut s = String::from(old_dir);
        s.push('/');
        s
    };
    let new_prefix: String = if new_dir.ends_with('/') {
        String::from(new_dir)
    } else {
        let mut s = String::from(new_dir);
        s.push('/');
        s
    };

    let mut old_entries = read_dir(&old_prefix);
    let mut new_entries = read_dir(&new_prefix);

    old_entries.sort_unstable();
    new_entries.sort_unstable();

    let mut old_index = 0usize;
    let mut new_index = 0usize;

    while old_index < old_entries.len() || new_index < new_entries.len() {
        let old_name = old_entries.get(old_index).map(|s| s.as_str());
        let new_name = new_entries.get(new_index).map(|s| s.as_str());

        match (old_name, new_name) {
            (Some(old), Some(new)) if old == new => {
                // Entry exists in both — compare.
                let mut old_path = old_prefix.clone();
                old_path.push_str(old);
                let mut new_path = new_prefix.clone();
                new_path.push_str(new);

                // Try to read as directory first (read_dir returns non-empty).
                let old_is_dir = !read_dir(&{
                    let mut p = old_path.clone(); p.push('/'); p
                }).is_empty() || old_path.ends_with('/');

                // Attempt to open as file; if it fails treat as directory.
                match open_file(&old_path) {
                    Ok(fd) => {
                        raw::raw_close(fd);
                        // It's a file — diff it.
                        diff_two_files(&old_path, &new_path, options, had_diff);
                    }
                    Err(_) => {
                        // Likely a directory — recurse.
                        if options.recursive {
                            write_stdout("Common subdirectories: ");
                            write_stdout(&old_path);
                            write_stdout(" and ");
                            write_stdout(&new_path);
                            write_stdout("\n");
                            diff_directories(&old_path, &new_path, options, had_diff);
                        }
                    }
                }
                let _ = old_is_dir;
                old_index += 1;
                new_index += 1;
            }
            (Some(old), Some(new)) if old < new => {
                // Entry only in old dir.
                write_stdout("Only in ");
                write_stdout(&old_prefix);
                write_stdout(": ");
                write_stdout(old);
                write_stdout("\n");
                *had_diff = true;
                old_index += 1;
            }
            (Some(_), Some(_)) => {
                // Entry only in new dir.
                write_stdout("Only in ");
                write_stdout(&new_prefix);
                write_stdout(": ");
                write_stdout(new_name.unwrap());
                write_stdout("\n");
                *had_diff = true;
                new_index += 1;
            }
            (Some(old), None) => {
                write_stdout("Only in ");
                write_stdout(&old_prefix);
                write_stdout(": ");
                write_stdout(old);
                write_stdout("\n");
                *had_diff = true;
                old_index += 1;
            }
            (None, Some(new)) => {
                write_stdout("Only in ");
                write_stdout(&new_prefix);
                write_stdout(": ");
                write_stdout(new);
                write_stdout("\n");
                *had_diff = true;
                new_index += 1;
            }
            (None, None) => break,
        }
    }
}

/// Diff two regular files, emitting output according to options.
/// Updates `had_diff` if differences are found.
fn diff_two_files(
    old_path: &str,
    new_path: &str,
    options: &Options,
    had_diff: &mut bool,
) {
    let old_data = match open_file(old_path) {
        Ok(fd) => { let d = read_fd_to_end(fd); raw::raw_close(fd); d }
        Err(errno) => {
            write_stderr("diff: ");
            write_stderr(old_path);
            write_stderr(": ");
            write_stderr(coreutils::strerror(errno));
            write_stderr("\n");
            return;
        }
    };
    let new_data = match open_file(new_path) {
        Ok(fd) => { let d = read_fd_to_end(fd); raw::raw_close(fd); d }
        Err(errno) => {
            write_stderr("diff: ");
            write_stderr(new_path);
            write_stderr(": ");
            write_stderr(coreutils::strerror(errno));
            write_stderr("\n");
            return;
        }
    };

    let old_lines: Vec<&str> = split_lines(&old_data);
    let new_lines: Vec<&str> = split_lines(&new_data);
    let edits = myers_diff(&old_lines, &new_lines, options.ignore_whitespace);
    let has_differences = edits.iter().any(|&(o, n)| o.is_none() || n.is_none());

    if !has_differences { return; }
    *had_diff = true;

    // POSIX: "diff %s %s %s\n" header when comparing within -r.
    write_stdout("diff ");
    match options.format {
        OutputFormat::Context  => write_stdout("-c "),
        OutputFormat::Unified  => write_stdout("-u "),
        OutputFormat::EdScript => write_stdout("-e "),
        OutputFormat::ReverseEdScript => write_stdout("-f "),
        OutputFormat::Default  => {}
    }
    write_stdout(old_path);
    write_stdout(" ");
    write_stdout(new_path);
    write_stdout("\n");

    emit_diff(&edits, &old_lines, &new_lines, old_path, new_path, options);
}

/// Emit the diff output for a computed edit list.
fn emit_diff(
    edits: &[(Option<usize>, Option<usize>)],
    old_lines: &[&str],
    new_lines: &[&str],
    old_name: &str,
    new_name: &str,
    options: &Options,
) {
    match options.format {
        OutputFormat::Default         => output_default(edits, old_lines, new_lines),
        OutputFormat::EdScript        => output_ed_script(edits, old_lines, new_lines),
        OutputFormat::ReverseEdScript => output_reverse_ed_script(edits, old_lines, new_lines),
        OutputFormat::Context         => output_context(edits, old_lines, new_lines, old_name, new_name, options.context_lines),
        OutputFormat::Unified         => output_unified(edits, old_lines, new_lines, old_name, new_name, options.context_lines),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    if arguments.len() < 2 {
        write_stderr("diff: requires exactly two file operands\n");
        raw::raw_exit(2);
    }

    let (options, old_name, new_name) = parse_options(&arguments[1..]);

    // ---------------------------------------------------------------------------
    // Recursive directory mode (-r).
    // ---------------------------------------------------------------------------
    if options.recursive {
        let mut had_diff = false;
        diff_directories(old_name, new_name, &options, &mut had_diff);
        raw::raw_exit(if had_diff { 1 } else { 0 });
    }

    // ---------------------------------------------------------------------------
    // Single-file mode.
    // ---------------------------------------------------------------------------
    let old_data = if old_name == "-" {
        read_stdin_to_end()
    } else {
        match open_file(old_name) {
            Ok(fd) => {
                let data = read_fd_to_end(fd);
                raw::raw_close(fd);
                data
            }
            Err(_) => {
                write_stderr("diff: ");
                write_stderr(old_name);
                write_stderr(": No such file or directory\n");
                raw::raw_exit(2);
            }
        }
    };

    let new_data = if new_name == "-" {
        read_stdin_to_end()
    } else {
        match open_file(new_name) {
            Ok(fd) => {
                let data = read_fd_to_end(fd);
                raw::raw_close(fd);
                data
            }
            Err(_) => {
                write_stderr("diff: ");
                write_stderr(new_name);
                write_stderr(": No such file or directory\n");
                raw::raw_exit(2);
            }
        }
    };

    let old_lines: Vec<&str> = split_lines(&old_data);
    let new_lines: Vec<&str> = split_lines(&new_data);
    let edits = myers_diff(&old_lines, &new_lines, options.ignore_whitespace);
    let has_differences = edits.iter().any(|&(o, n)| o.is_none() || n.is_none());

    if !has_differences {
        raw::raw_exit(0);
    }

    emit_diff(&edits, &old_lines, &new_lines, old_name, new_name, &options);
    raw::raw_exit(1)
}

/// Split bytes into lines, keeping the trailing '\n' in each line.
/// The last line is included even if it has no trailing newline.
fn split_lines(data: &[u8]) -> Vec<&str> {
    let mut lines: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for (index, &byte) in data.iter().enumerate() {
        if byte == b'\n' {
            if let Ok(s) = core::str::from_utf8(&data[start..=index]) {
                lines.push(s);
            }
            start = index + 1;
        }
    }
    if start < data.len() {
        if let Ok(s) = core::str::from_utf8(&data[start..]) {
            lines.push(s);
        }
    }
    lines
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(2)
}
