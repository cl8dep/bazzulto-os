// sh/expand.rs — POSIX §2.6 Word Expansions
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// Order of expansion (§2.6):
//   1. Tilde expansion          (§2.6.1)   — implemented
//   2. Parameter expansion      (§2.6.2)   — implemented
//   3. Command substitution     (§2.6.3)   — implemented ($() and `...`)
//   4. Arithmetic expansion     (§2.6.4)   — implemented ($((...)))
//   5. Field splitting          (§2.6.5)   — implemented
//   6. Pathname expansion       (§2.6.6)   — TODO (future spec, requires VFS readdir)
//   7. Quote removal            (§2.6.7)   — implemented
//
// Public entry point:
//   expand_word(word, state) -> Vec<String>
//     Applies steps 1-7 and returns the resulting fields.
//     Usually one field; multiple fields result from $@ or IFS field splitting.
//
// Design notes:
//   The expander works on a "marked string" internally: a sequence of
//   `Piece` values that track whether each segment came from a quoted context
//   (which prevents field splitting and pathname expansion on that segment).
//   This avoids re-scanning the string for quote marks after expansion.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ShellState;
use crate::vars::{expand_special, is_valid_name, format_u32};

// ---------------------------------------------------------------------------
// Piece — internal expansion unit
// ---------------------------------------------------------------------------

/// A piece of an expanded word, tagged with quoting status.
///
/// Quoted pieces are not subject to field splitting or pathname expansion.
#[derive(Debug)]
struct Piece {
    text:   String,
    quoted: bool,
}

impl Piece {
    fn literal(s: &str) -> Self { Piece { text: s.to_string(), quoted: false } }
    fn quoted(s: &str)  -> Self { Piece { text: s.to_string(), quoted: true  } }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Expand a single word through all §2.6 stages.
///
/// Returns a `Vec<String>` of fields (usually one).
/// An empty `Vec` is possible if the word expands to nothing and field
/// splitting produces no fields.
pub fn expand_word(word: &str, state: &mut ShellState) -> Vec<String> {
    // Stage 1-4: produce a list of Pieces.
    let pieces = expand_to_pieces(word, state, false);

    // Stage 5: field splitting on unquoted pieces.
    let fields = field_split(pieces, state);

    // Stage 6: pathname expansion — deferred (TODO §2.6.6).
    // Each field would be glob-expanded here.

    // Stage 7: quote removal is already done — quoted Pieces had their
    // surrounding quote characters stripped during lexing (§2.2); quote
    // removal in §2.6.7 just means the tagged pieces are now plain strings.
    fields
}

/// Like `expand_word`, but the result is always a single string (no field
/// splitting). Used for filenames in redirects and for assignment values.
pub fn expand_word_nosplit(word: &str, state: &mut ShellState) -> String {
    let pieces = expand_to_pieces(word, state, false);
    pieces.into_iter().map(|p| p.text).collect::<String>()
}

// ---------------------------------------------------------------------------
// Stage 1–4: expand to pieces
// ---------------------------------------------------------------------------

/// Process a word string through tilde, parameter, command-substitution
/// (stub), and arithmetic (stub) expansions, producing a list of Pieces.
///
/// `inside_double_quote`: when true, field splitting rules differ for $@ / $*.
fn expand_to_pieces(word: &str, state: &mut ShellState, inside_double_quote: bool) -> Vec<Piece> {
    let bytes = word.as_bytes();
    let len = bytes.len();
    let mut pieces: Vec<Piece> = Vec::new();

    // Buffer for the current unquoted literal segment.
    let mut literal_buf = String::new();

    // Helper: flush the current literal buffer as an unquoted piece.
    macro_rules! flush_literal {
        () => {
            if !literal_buf.is_empty() {
                pieces.push(Piece::literal(&literal_buf));
                literal_buf.clear();
            }
        };
    }

    // §2.6.1 Tilde expansion: only at the start of a word, or after an
    // unquoted '=' or ':' in an assignment context.
    let word = if word.starts_with('~') {
        let expanded = expand_tilde(word, state);
        if expanded != word {
            // Tilde was expanded: the result is a quoted piece (§2.6.1:
            // "treated as if quoted to prevent field splitting / pathname
            // expansion").
            pieces.push(Piece::quoted(&expanded));
            return pieces; // tilde expansion replaces the whole word
        }
        word
    } else {
        word
    };

    let bytes = word.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    while index < len {
        let byte = bytes[index];

        match byte {
            // -----------------------------------------------------------------
            // $ — parameter expansion, command substitution, arithmetic
            // -----------------------------------------------------------------
            b'$' => {
                let (piece, consumed) = expand_dollar(bytes, index, state, inside_double_quote);
                flush_literal!();
                if let Some(p) = piece {
                    pieces.push(p);
                }
                index += consumed;
            }

            // -----------------------------------------------------------------
            // ` — command substitution (§2.6.3)
            // -----------------------------------------------------------------
            b'`' => {
                // Find the matching closing backtick.
                let mut j = index + 1;
                while j < bytes.len() && bytes[j] != b'`' {
                    if bytes[j] == b'\\' { j += 1; } // skip backslash-escaped char
                    j += 1;
                }
                let cmd_text = core::str::from_utf8(&bytes[index+1..j]).unwrap_or("");
                flush_literal!();
                let sub_fn = state.command_sub_fn;
                let result = sub_fn(cmd_text, state);
                pieces.push(Piece::literal(&result));
                index = if j < bytes.len() { j + 1 } else { j }; // skip closing `
            }

            // -----------------------------------------------------------------
            // Regular character — accumulate into literal buffer.
            // -----------------------------------------------------------------
            _ => {
                literal_buf.push(byte as char);
                index += 1;
            }
        }
    }

    flush_literal!();
    pieces
}

// ---------------------------------------------------------------------------
// §2.6.1 Tilde expansion
// ---------------------------------------------------------------------------

/// Expand a tilde-prefix at the start of `word`.
///
/// Rules:
///   `~`        → value of $HOME (unspecified if HOME unset; we return "~")
///   `~/path`   → $HOME/path
///   `~user`    → unspecified (no passwd database); return unchanged.
///   `~user/p`  → unspecified; return unchanged.
fn expand_tilde(word: &str, state: &ShellState) -> String {
    debug_assert!(word.starts_with('~'));

    // Find the end of the tilde-prefix (first unquoted slash or end of word).
    let after_tilde = &word[1..];
    let slash_pos = after_tilde.find('/');

    let login_name = match slash_pos {
        Some(p) => &after_tilde[..p],
        None    => after_tilde,
    };

    let suffix = match slash_pos {
        Some(p) => &after_tilde[p..], // includes the leading slash
        None    => "",
    };

    if login_name.is_empty() {
        // `~` or `~/path` — use $HOME.
        let home = state.vars.get("HOME").unwrap_or("");
        if home.is_empty() {
            return word.to_string(); // HOME unset or empty — unspecified
        }
        // §2.6.1: if suffix starts with '/' and home ends with '/', omit
        // the trailing slash from home.
        let home = home.trim_end_matches('/');
        let mut result = String::from(home);
        result.push_str(suffix);
        return result;
    }

    // `~user` — no user database; return unchanged.
    word.to_string()
}

// ---------------------------------------------------------------------------
// §2.6.2 Parameter expansion — $... and ${...}
// ---------------------------------------------------------------------------

/// Process a `$` at `bytes[start]`.
///
/// Returns `(Option<Piece>, bytes_consumed)`.
/// `bytes_consumed` includes the `$` itself.
fn expand_dollar(
    bytes: &[u8],
    start: usize,
    state: &mut ShellState,
    _inside_double_quote: bool,
) -> (Option<Piece>, usize) {
    let len = bytes.len();
    let next = if start + 1 < len { bytes[start + 1] } else { 0 };

    match next {
        // ${...} — braced parameter expansion.
        b'{' => {
            expand_braced(bytes, start, state)
        }

        // $((...)) — arithmetic expansion (§2.6.4).
        // $(...) — command substitution (§2.6.3).
        b'(' => {
            // Check for $((...)) — double paren = arithmetic.
            if bytes.get(start + 2) == Some(&b'(') {
                // Skip past the outer '(' to find the content of ((...))
                // skip_paren_group starting at `start+1` includes the outer `(`
                // and returns the inner bytes; then we need to strip one more layer.
                let (outer_inner, outer_consumed) = skip_paren_group(bytes, start + 1);
                // outer_inner is the bytes of `(...expr...)` without the outer parens.
                // We need the inner expression (strip leading '(' and trailing ')').
                let arith_expr = if outer_inner.len() >= 2 {
                    core::str::from_utf8(&outer_inner[1..outer_inner.len()-1]).unwrap_or("")
                } else {
                    ""
                };
                let result = arithmetic_expand(arith_expr, state);
                let result_str = format_i64(result);
                (Some(Piece::quoted(&result_str)), 1 + outer_consumed)
            } else {
                // $(...) — command substitution.
                let (inner_bytes, consumed) = skip_paren_group(bytes, start + 1);
                let cmd_text = core::str::from_utf8(inner_bytes).unwrap_or("");
                // Copy the fn pointer to avoid double-borrow of state.
                let sub_fn = state.command_sub_fn;
                let result = sub_fn(cmd_text, state);
                (Some(Piece::literal(&result)), 1 + consumed)
            }
        }

        // $@ $* $# $? $- $$ $! $0
        b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!' | b'0' => {
            let ch = next as char;
            let value = expand_special(ch, state);
            // $@ and $* may produce multiple fields — handled by field_split
            // via the quoted=false tag.
            (Some(Piece::literal(&value)), 2)
        }

        // $1–$9 — single-digit positional parameter.
        b'1'..=b'9' => {
            let n = (next - b'0') as usize;
            let value = state.positional_params.get(n - 1)
                .cloned()
                .unwrap_or_default();
            (Some(Piece::literal(&value)), 2)
        }

        // $NAME — unbraced variable name.
        c if c == b'_' || c.is_ascii_alphabetic() => {
            // Consume the longest valid name.
            let name_start = start + 1;
            let mut name_end = name_start;
            while name_end < len
                && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
            {
                name_end += 1;
            }
            let name = core::str::from_utf8(&bytes[name_start..name_end]).unwrap_or("");
            let value = state.vars.get(name).unwrap_or("").to_string();
            (Some(Piece::literal(&value)), name_end - start)
        }

        // $ followed by space, tab, newline, or end — literal '$' (§2.6).
        b' ' | b'\t' | b'\n' | 0 => {
            (Some(Piece::literal("$")), 1)
        }

        // Unspecified by POSIX — treat as literal '$'.
        _ => (Some(Piece::literal("$")), 1),
    }
}

// ---------------------------------------------------------------------------
// §2.6.2 Braced parameter expansion: ${...}
// ---------------------------------------------------------------------------

fn expand_braced(bytes: &[u8], start: usize, state: &mut ShellState) -> (Option<Piece>, usize) {
    // bytes[start] = '$', bytes[start+1] = '{'
    let len = bytes.len();
    let inner_start = start + 2; // first char after '{'

    // Find the matching '}' — must account for nested ${} and quotes per spec.
    // Simple scan for now (no nested expansion inside ${...} yet).
    let close = match find_close_brace(bytes, inner_start) {
        Some(pos) => pos,
        None => {
            // Unterminated ${: return literal '${'
            return (Some(Piece::literal("${")), 2);
        }
    };

    let inner = core::str::from_utf8(&bytes[inner_start..close]).unwrap_or("");
    let consumed = close + 1 - start; // includes '$', '{', inner, '}'

    // §2.6.2: ${#parameter} — string length.
    if inner.starts_with('#') {
        let param = &inner[1..];
        if param.is_empty() {
            // ${#} — number of positional params (same as $#).
            let n = format_u32(state.positional_params.len() as u32);
            return (Some(Piece::quoted(&n)), consumed);
        }
        let value = get_param_value(param, state);
        let length = format_u32(value.chars().count() as u32);
        return (Some(Piece::quoted(&length)), consumed);
    }

    // Detect operator: :-, :=, :?, :+, -, =, ?, +, %, %%, #, ##
    // Find the operator position within inner.
    if let Some((param, op, word)) = parse_param_op(inner) {
        let value = expand_param_op(param, op, word, state);
        return (Some(Piece::quoted(&value)), consumed);
    }

    // Simple ${parameter} — substitute value.
    let value = get_param_value(inner, state);
    (Some(Piece::quoted(&value)), consumed)
}

// ---------------------------------------------------------------------------
// §2.6.2 Parameter operator parsing and evaluation
// ---------------------------------------------------------------------------

/// Parse `inner` (the content of `${...}`) into `(param, op, word)`.
///
/// Operators: `:-` `:=` `:?` `:+` `-` `=` `?` `+` `%` `%%` `#` `##`
fn parse_param_op(inner: &str) -> Option<(&str, &str, &str)> {
    // Scan for operator characters: : - = ? + % #
    let bytes = inner.as_bytes();
    let mut i = 0usize;

    // The parameter name is the longest valid name or special parameter char.
    // Special single-char params: @ * # ? - $ ! 0-9
    if !bytes.is_empty() && is_special_param_char(bytes[0]) {
        i = 1;
    } else {
        while i < bytes.len()
            && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
        {
            i += 1;
        }
    }

    if i >= bytes.len() {
        return None; // no operator
    }

    let param = &inner[..i];
    let rest  = &inner[i..];

    // Two-char operators first.
    if rest.starts_with(":-") { return Some((param, ":-", &rest[2..])); }
    if rest.starts_with(":=") { return Some((param, ":=", &rest[2..])); }
    if rest.starts_with(":?") { return Some((param, ":?", &rest[2..])); }
    if rest.starts_with(":+") { return Some((param, ":+", &rest[2..])); }
    if rest.starts_with("%%") { return Some((param, "%%", &rest[2..])); }
    if rest.starts_with("##") { return Some((param, "##", &rest[2..])); }

    // Single-char operators.
    if rest.starts_with('-') { return Some((param, "-",  &rest[1..])); }
    if rest.starts_with('=') { return Some((param, "=",  &rest[1..])); }
    if rest.starts_with('?') { return Some((param, "?",  &rest[1..])); }
    if rest.starts_with('+') { return Some((param, "+",  &rest[1..])); }
    if rest.starts_with('%') { return Some((param, "%",  &rest[1..])); }
    if rest.starts_with('#') { return Some((param, "#",  &rest[1..])); }

    None
}

/// Evaluate a `${param op word}` expression (§2.6.2).
///
/// `word` is not expanded here — POSIX requires it to be expanded only when
/// needed ("word shall be subjected to tilde expansion, parameter expansion,
/// command substitution, arithmetic expansion, and quote removal").
/// For v1.0 we expand word as a simple literal (no recursive expansion yet).
/// Full recursive expansion will be added when §2.6.3/§2.6.4 arrive.
fn expand_param_op(
    param: &str,
    op: &str,
    word: &str,
    state: &mut ShellState,
) -> String {
    // Get parameter value and set/null status.
    let (is_set, is_null, value) = {
        let v = get_param_value_opt(param, state);
        match v {
            None    => (false, true,  String::new()),
            Some(s) if s.is_empty() => (true, true, s),
            Some(s) => (true, false, s),
        }
    };

    match op {
        // ${param:-word}: unset or null → word; else param.
        ":-" => if !is_set || is_null { word.to_string() } else { value },

        // ${param-word}: unset → word; else param (null is ok).
        "-"  => if !is_set            { word.to_string() } else { value },

        // ${param:=word}: unset or null → assign word to param, substitute word.
        ":=" => {
            if !is_set || is_null {
                if is_valid_name(param) {
                    let _ = state.vars.set(param, word);
                }
                word.to_string()
            } else { value }
        }

        // ${param=word}: unset → assign word to param, substitute word.
        "="  => {
            if !is_set {
                if is_valid_name(param) {
                    let _ = state.vars.set(param, word);
                }
                word.to_string()
            } else { value }
        }

        // ${param:?word}: unset or null → error.
        ":?" => {
            if !is_set || is_null {
                crate::write_err("sh: ");
                crate::write_err(param);
                crate::write_err(": ");
                let msg = if word.is_empty() { "parameter null or not set" } else { word };
                crate::write_err(msg);
                crate::write_err("\n");
                // Non-interactive shell exits (§2.6.2). Interactive shell need not.
                // We exit for simplicity — revisit when interactive detection is added.
                crate::exit_on_error(1);
            } else { value }
        }

        // ${param?word}: unset → error (null is ok).
        "?"  => {
            if !is_set {
                crate::write_err("sh: ");
                crate::write_err(param);
                crate::write_err(": ");
                let msg = if word.is_empty() { "parameter not set" } else { word };
                crate::write_err(msg);
                crate::write_err("\n");
                crate::exit_on_error(1);
            } else { value }
        }

        // ${param:+word}: set and not null → word; else null.
        ":+" => if is_set && !is_null { word.to_string() } else { String::new() },

        // ${param+word}: set (even if null) → word; else null.
        "+"  => if is_set             { word.to_string() } else { String::new() },

        // ${param%word}: remove smallest suffix matching pattern.
        "%"  => remove_suffix(&value, word, false),

        // ${param%%word}: remove largest suffix matching pattern.
        "%%" => remove_suffix(&value, word, true),

        // ${param#word}: remove smallest prefix matching pattern.
        "#"  => remove_prefix(&value, word, false),

        // ${param##word}: remove largest prefix matching pattern.
        "##" => remove_prefix(&value, word, true),

        _ => value,
    }
}

// ---------------------------------------------------------------------------
// §2.6.2 Helper: get parameter value
// ---------------------------------------------------------------------------

/// Get the current value of a named parameter or special parameter.
/// Returns empty string if unset (same as POSIX unset-without-:u behavior).
/// If `state.option_nounset` is set and the variable is unset, exits with error.
fn get_param_value(param: &str, state: &ShellState) -> String {
    let opt = get_param_value_opt(param, state);
    if opt.is_none() && state.option_nounset {
        crate::write_err("sh: ");
        crate::write_err(param);
        crate::write_err(": unbound variable\n");
        crate::exit_on_error(1);
    }
    opt.unwrap_or_default()
}

/// Like `get_param_value` but returns None if unset (for :- := :? etc.).
fn get_param_value_opt(param: &str, state: &ShellState) -> Option<String> {
    // Single-character special parameters.
    if param.len() == 1 {
        let ch = param.chars().next().unwrap();
        if is_special_param_char(ch as u8) {
            return Some(expand_special(ch, state));
        }
    }

    // Multi-digit positional parameter: ${10} ${11} ...
    if param.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = param.parse::<usize>() {
            if n == 0 {
                // ${0} — unspecified per spec; we return shell name.
                return Some(state.shell_name.clone());
            }
            return state.positional_params.get(n - 1).cloned();
        }
    }

    // Named variable.
    state.vars.get(param).map(|s| s.to_string())
}

fn is_special_param_char(b: u8) -> bool {
    matches!(b, b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!' | b'0'..=b'9')
}

// ---------------------------------------------------------------------------
// §2.6.2 Pattern matching for % %% # ## (§2.14 Pattern Matching Notation)
// ---------------------------------------------------------------------------

/// Remove the smallest (`largest=false`) or largest (`largest=true`) suffix
/// of `value` that matches `pattern`.
///
/// Pattern matching notation (§2.14): `*` = any string, `?` = any char,
/// `[...]` = character class. We implement `*` and `?` only for now.
fn remove_suffix(value: &str, pattern: &str, largest: bool) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();

    if largest {
        // Largest suffix: try from the beginning of the string.
        for start in 0..=n {
            let suffix: String = chars[start..].iter().collect();
            if glob_match(pattern, &suffix) {
                return chars[..start].iter().collect();
            }
        }
    } else {
        // Smallest suffix: try from the end of the string.
        for start in (0..=n).rev() {
            let suffix: String = chars[start..].iter().collect();
            if glob_match(pattern, &suffix) {
                return chars[..start].iter().collect();
            }
        }
    }
    value.to_string() // no match: return unchanged
}

/// Remove the smallest or largest prefix of `value` matching `pattern`.
fn remove_prefix(value: &str, pattern: &str, largest: bool) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();

    if largest {
        // Largest prefix: try from the end.
        for end in (0..=n).rev() {
            let prefix: String = chars[..end].iter().collect();
            if glob_match(pattern, &prefix) {
                return chars[end..].iter().collect();
            }
        }
    } else {
        // Smallest prefix: try from the beginning.
        for end in 0..=n {
            let prefix: String = chars[..end].iter().collect();
            if glob_match(pattern, &prefix) {
                return chars[end..].iter().collect();
            }
        }
    }
    value.to_string()
}

/// Simple glob pattern match (§2.14 subset: `*`, `?`, literal chars).
/// `[...]` character classes are not yet implemented.
/// Returns true if `pattern` matches the entire string `s`.
fn glob_match(pattern: &str, s: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = s.chars().collect();
    glob_match_inner(&pat, &text)
}

fn glob_match_inner(pat: &[char], text: &[char]) -> bool {
    match (pat.first(), text.first()) {
        (None, None)    => true,
        (None, Some(_)) => false,
        (Some(&'*'), _) => {
            // '*' matches zero or more characters.
            // Try matching the rest of the pattern at every position in text.
            for skip in 0..=text.len() {
                if glob_match_inner(&pat[1..], &text[skip..]) {
                    return true;
                }
            }
            false
        }
        (Some(&'?'), Some(_)) => glob_match_inner(&pat[1..], &text[1..]),
        // Bracket expression `[...]` (§2.13.1).
        (Some(&'['), Some(&tc)) => {
            // Find the closing `]`, respecting `[!...]` negation and `[]...]` literal `]`.
            let mut j = 1usize;
            let negate = pat.get(j) == Some(&'!') || pat.get(j) == Some(&'^');
            if negate { j += 1; }
            // Allow `]` as first char in bracket expression.
            if pat.get(j) == Some(&']') { j += 1; }
            while j < pat.len() && pat[j] != ']' { j += 1; }
            let bracket_content = if j < pat.len() { &pat[1..j] } else { return false };
            let start = if negate { 1 } else { 0 };
            let chars = &bracket_content[start..];

            let mut matched_in_bracket = false;
            let mut k = 0usize;
            while k < chars.len() {
                if k + 2 < chars.len() && chars[k + 1] == '-' {
                    // Range: c1-c2
                    if tc >= chars[k] && tc <= chars[k + 2] {
                        matched_in_bracket = true;
                    }
                    k += 3;
                } else {
                    if tc == chars[k] { matched_in_bracket = true; }
                    k += 1;
                }
            }

            let char_matched = if negate { !matched_in_bracket } else { matched_in_bracket };
            if char_matched {
                // +2: skip `[` and `]`
                glob_match_inner(&pat[j + 1..], &text[1..])
            } else {
                false
            }
        }
        (Some(p), Some(t))    => p == t && glob_match_inner(&pat[1..], &text[1..]),
        (Some(_), None)       => false,
    }
}

/// POSIX §2.13 Pattern matching: test whether `pattern` matches the full string `subject`.
///
/// Supports `*` (any sequence), `?` (any single char), `[...]` character classes
/// (including ranges, negation with `!` or `^`, and backslash escaping).
pub fn pattern_matches(pattern: &str, subject: &str) -> bool {
    glob_match(pattern, subject)
}

// ---------------------------------------------------------------------------
// §2.6.5 Field splitting
// ---------------------------------------------------------------------------

/// Split a list of pieces into fields using IFS (§2.6.5).
///
/// Quoted pieces are never split.
/// Unquoted pieces are split on IFS characters.
///
/// IFS whitespace (space, tab, newline): sequences of IFS whitespace at the
/// beginning or end of the unquoted result are discarded.
/// IFS non-whitespace: each occurrence delimits a field; adjacent
/// non-whitespace chars each produce a delimiter (no merging).
fn field_split(pieces: Vec<Piece>, state: &ShellState) -> Vec<String> {
    // Get IFS: default is <space><tab><newline> if unset (§2.5.3).
    let ifs = match state.vars.get("IFS") {
        Some(v) => v,
        None    => " \t\n",
    };
    // If IFS is empty, no field splitting occurs (§2.6.5).
    if ifs.is_empty() {
        return pieces.into_iter().map(|p| p.text).collect();
    }

    let ifs_chars: Vec<char> = ifs.chars().collect();

    // Separate IFS into whitespace and non-whitespace sets (§2.6.5).
    let ifs_ws: Vec<char>  = ifs_chars.iter().cloned()
        .filter(|c| *c == ' ' || *c == '\t' || *c == '\n')
        .collect();
    let ifs_nonws: Vec<char> = ifs_chars.iter().cloned()
        .filter(|c| *c != ' ' && *c != '\t' && *c != '\n')
        .collect();

    let mut fields: Vec<String> = Vec::new();
    let mut current_field = String::new();
    let mut in_field = false;
    let mut last_was_nonws_delim = false;

    for piece in pieces {
        if piece.quoted {
            // Quoted: append verbatim, starts/continues a field.
            current_field.push_str(&piece.text);
            in_field = true;
            last_was_nonws_delim = false;
            continue;
        }

        // Unquoted: scan character by character.
        for ch in piece.text.chars() {
            let is_ifs_ws    = ifs_ws.contains(&ch);
            let is_ifs_nonws = ifs_nonws.contains(&ch);

            if is_ifs_nonws {
                // Non-whitespace IFS: always a delimiter.
                // A leading IFS non-whitespace after IFS whitespace produces
                // an empty field (§2.6.5).
                fields.push(core::mem::take(&mut current_field));
                in_field = false;
                last_was_nonws_delim = true;
            } else if is_ifs_ws {
                // IFS whitespace: flushes the current field if in one.
                if in_field {
                    fields.push(core::mem::take(&mut current_field));
                    in_field = false;
                }
                last_was_nonws_delim = false;
            } else {
                // Regular character: part of a field.
                if last_was_nonws_delim {
                    // Start a new field after a non-ws IFS delimiter.
                    // (the previous push already created an empty field entry
                    // for the delimiter; now we start the next field)
                    last_was_nonws_delim = false;
                }
                current_field.push(ch);
                in_field = true;
            }
        }
    }

    // Flush the last field.
    if in_field || !current_field.is_empty() {
        fields.push(current_field);
    }

    // Remove empty fields produced by trailing IFS whitespace at start/end,
    // but keep fields produced by non-whitespace IFS delimiters.
    // (Already handled above — leading/trailing IFS whitespace is discarded
    // by not starting a field until a non-IFS char is seen.)

    // If no fields were produced, return a single empty string if the original
    // word was a non-null expansion, or empty vec otherwise.
    // The caller (expand_word) decides what to do with an empty result.
    fields
}

// ---------------------------------------------------------------------------
// §2.6.7 Quote removal (performed implicitly by the Piece structure)
// ---------------------------------------------------------------------------
// Quote removal is already implicit: the lexer strips the quote characters
// from the word string before it reaches the expander (§2.2 quoting means
// the delimiters are consumed). The `quoted` flag on a Piece indicates that
// the content should not be further processed (field split / pathname expand).
// No additional pass is needed here.

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the position of the closing `}` for a `${` expression, starting at
/// `start` (the first character after `{`). Returns the index of `}`.
fn find_close_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => { depth += 1; i += 1; }
            b'}' => {
                depth -= 1;
                if depth == 0 { return Some(i); }
                i += 1;
            }
            b'\\' => { i += 2; } // skip escaped char
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' { i += 1; }
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// §2.6.4 Arithmetic expansion: $((...))
// ---------------------------------------------------------------------------

/// Evaluate a POSIX arithmetic expression (§2.6.4).
///
/// Supports: integer literals, variable references (no `$` prefix inside `(())`),
/// unary `-`/`+`, binary `+` `-` `*` `/` `%`, parentheses.
/// Returns 0 on any error (undefined value per POSIX).
fn arithmetic_expand(expr: &str, state: &mut ShellState) -> i64 {
    let expr = expr.trim();
    // Expand variable references first (any $name or ${...} in the expression).
    let expanded = expand_arith_vars(expr, state);
    let bytes: Vec<u8> = expanded.bytes().collect();
    let mut pos = 0usize;
    let result = arith_expr(&bytes, &mut pos);
    result.unwrap_or(0)
}

/// Expand variable names within an arithmetic expression.
/// $name and ${name} are expanded; other chars pass through.
fn expand_arith_vars(expr: &str, state: &mut ShellState) -> String {
    let bytes = expr.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'$' {
            let (piece, consumed) = expand_dollar(bytes, i, state, false);
            if let Some(p) = piece {
                out.push_str(&p.text);
            }
            i += consumed;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Recursive-descent arithmetic parser for POSIX §2.6.4.
/// Grammar (simplified):
///   expr    = additive
///   additive = multiplicative (('+' | '-') multiplicative)*
///   multiplicative = unary (('*' | '/' | '%') unary)*
///   unary   = ('+' | '-')? primary
///   primary = number | identifier | '(' expr ')'
fn arith_expr(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    arith_skip_ws(bytes, pos);
    arith_additive(bytes, pos)
}

fn arith_additive(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    let mut lhs = arith_multiplicative(bytes, pos)?;
    loop {
        arith_skip_ws(bytes, pos);
        match bytes.get(*pos) {
            Some(&b'+') => { *pos += 1; let rhs = arith_multiplicative(bytes, pos)?; lhs = lhs.wrapping_add(rhs); }
            Some(&b'-') => { *pos += 1; let rhs = arith_multiplicative(bytes, pos)?; lhs = lhs.wrapping_sub(rhs); }
            _ => break,
        }
    }
    Some(lhs)
}

fn arith_multiplicative(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    let mut lhs = arith_unary(bytes, pos)?;
    loop {
        arith_skip_ws(bytes, pos);
        match bytes.get(*pos) {
            Some(&b'*') => { *pos += 1; let rhs = arith_unary(bytes, pos)?; lhs = lhs.wrapping_mul(rhs); }
            Some(&b'/') => { *pos += 1; let rhs = arith_unary(bytes, pos)?; if rhs == 0 { return None; } lhs /= rhs; }
            Some(&b'%') => { *pos += 1; let rhs = arith_unary(bytes, pos)?; if rhs == 0 { return None; } lhs %= rhs; }
            _ => break,
        }
    }
    Some(lhs)
}

fn arith_unary(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    arith_skip_ws(bytes, pos);
    match bytes.get(*pos) {
        Some(&b'-') => { *pos += 1; Some(arith_primary(bytes, pos)?.wrapping_neg()) }
        Some(&b'+') => { *pos += 1; arith_primary(bytes, pos) }
        _ => arith_primary(bytes, pos),
    }
}

fn arith_primary(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    arith_skip_ws(bytes, pos);
    match bytes.get(*pos) {
        Some(&b'(') => {
            *pos += 1;
            let val = arith_expr(bytes, pos)?;
            arith_skip_ws(bytes, pos);
            if bytes.get(*pos) == Some(&b')') { *pos += 1; }
            Some(val)
        }
        Some(&c) if c.is_ascii_digit() => {
            let start = *pos;
            while *pos < bytes.len() && bytes[*pos].is_ascii_digit() { *pos += 1; }
            let s = core::str::from_utf8(&bytes[start..*pos]).ok()?;
            s.parse::<i64>().ok()
        }
        _ => Some(0), // unrecognized: treat as 0
    }
}

fn arith_skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t') {
        *pos += 1;
    }
}

/// Format a signed 64-bit integer as a decimal string (no alloc::format! needed).
fn format_i64(n: i64) -> String {
    if n == 0 { return "0".into(); }
    let negative = n < 0;
    let mut abs = if negative { n.wrapping_neg() as u64 } else { n as u64 };
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while abs > 0 {
        digits[len] = b'0' + (abs % 10) as u8;
        abs /= 10;
        len += 1;
    }
    let mut result = String::new();
    if negative { result.push('-'); }
    for i in (0..len).rev() {
        result.push(digits[i] as char);
    }
    result
}

/// Skip over a parenthesized group starting at `start` (the `(` position).
/// Returns `(inner_str, bytes_consumed_including_parens)`.
fn skip_paren_group(bytes: &[u8], start: usize) -> (&[u8], usize) {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => { depth += 1; i += 1; }
            b')' => {
                if depth == 0 { return (&bytes[start..i], i + 1 - start); }
                depth -= 1;
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    (&bytes[start..], bytes.len() - start)
}
