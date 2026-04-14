// sh/lexer.rs — POSIX §2.3 Token Recognition + §2.2 Quoting + §2.4 Reserved Words
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// §2.2 Quoting (implemented):
//   §2.2.1  Backslash: preserves literal value of next char.
//           \<newline> = line continuation (both removed, not a token separator).
//   §2.2.2  Single-quotes: all characters literal, no escapes inside.
//   §2.2.3  Double-quotes: $, `, \ retain special meaning; rest is literal.
//           \ is special only before: $ ` \ <newline> "
//   §2.2.4  Dollar-single-quotes $'...': backslash-escape sequences processed.
//           Supported: \' \" \\ \a \b \e \f \n \r \t \v \xXX \ddd
//
// §2.3 Token Recognition (implemented):
//   - Operator tokens: | || & && ; ;; < > >> >| >& <& <> <<  ( )
//     (|| && ;; ( ) recognized but compound-command use deferred to future spec)
//     (<< here-document deferred to parser / REPL continuation-line logic)
//   - Word tokens: anything not an operator or blank.
//   - # starts a comment only when not inside a word (rule 10).
//   - $ and ` outside quoting introduce expansions (§2.6, future spec) —
//     currently tokenized as part of words.
//   - [n]redir-op: optional decimal fd number immediately before a redir op.
//
// §2.4 Reserved Words (implemented):
//   Reserved words are recognized as Token::Word by the lexer.
//   The parser elevates them to reserved-word context when appropriate.
//   List: ! { } case do done elif else esac fi for if in then until while
//
// §2.7 Redirection operators (implemented):
//   [n]<word     stdin redirect        (default fd 0)
//   [n]>word     stdout redirect       (default fd 1)
//   [n]>>word    stdout append         (default fd 1)
//   [n]>|word    stdout noclobber      (default fd 1, same as > for now)
//   [n]>&word    dup output fd         (default fd 1)
//   [n]<&word    dup input fd          (default fd 0)
//   [n]<>word    open read/write       (default fd 0)
//   <<word       here-document         (emitted as token; REPL reads body)
//   <<-word      here-document (strip tabs)

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/// POSIX shell token types.
///
/// Redirect tokens carry an optional explicit fd number (§2.7: `[n]redir-op`).
/// `None` means "use the default fd for this operator".
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A word: command name, argument, assignment, or filename.
    Word(String),

    // --- List / pipeline operators ---

    /// `|` — pipeline connector (§2.9.2).
    Pipe,
    /// `||` — OR list operator (§2.9.3, future).
    PipeOr,
    /// `&` — background execution (§2.9.3, future).
    Amp,
    /// `&&` — AND list operator (§2.9.3, future).
    AmpAmp,
    /// `;` — sequential command separator (§2.9.3).
    Semi,
    /// `;;` — case clause terminator (§2.9.4, future).
    SemiSemi,

    // --- Redirection operators (§2.7) ---

    /// `[n]<` — redirect stdin (default fd 0).
    RedirIn(Option<u32>),
    /// `[n]>` — redirect stdout, truncate (default fd 1).
    RedirOut(Option<u32>),
    /// `[n]>>` — redirect stdout, append (default fd 1).
    RedirAppend(Option<u32>),
    /// `[n]>|` — redirect stdout, no-clobber override (default fd 1).
    RedirOutNoclobber(Option<u32>),
    /// `[n]>&` — duplicate output fd (default fd 1).
    RedirDupOut(Option<u32>),
    /// `[n]<&` — duplicate input fd (default fd 0).
    RedirDupIn(Option<u32>),
    /// `[n]<>` — open read/write (default fd 0).
    RedirReadWrite(Option<u32>),
    /// `[n]<<word` — here-document (default fd 0). Bool = strip_tabs (<<-).
    HereDoc(Option<u32>, bool),

    // --- Compound command delimiters ---

    /// `(` — subshell start (§2.9.4, future).
    LParen,
    /// `)` — subshell end / case pattern end (§2.9.4, future).
    RParen,
    /// `\n` — newline; acts as command terminator in compound commands.
    Newline,
}

impl Token {
    pub fn as_word(&self) -> Option<&str> {
        if let Token::Word(s) = self { Some(s) } else { None }
    }

    /// Returns true if this token is any redirect operator.
    pub fn is_redirect_op(&self) -> bool {
        matches!(self,
            Token::RedirIn(_) | Token::RedirOut(_) | Token::RedirAppend(_) |
            Token::RedirOutNoclobber(_) | Token::RedirDupOut(_) | Token::RedirDupIn(_) |
            Token::RedirReadWrite(_) | Token::HereDoc(_, _)
        )
    }
}

// ---------------------------------------------------------------------------
// §2.4 Reserved Words
// ---------------------------------------------------------------------------

pub const RESERVED_WORDS: &[&str] = &[
    "!", "{", "}", "case", "do", "done", "elif", "else",
    "esac", "fi", "for", "if", "in", "then", "until", "while",
];

pub fn is_reserved_word(word: &str) -> bool {
    RESERVED_WORDS.contains(&word)
}

// ---------------------------------------------------------------------------
// Lexer state machine
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
enum QuoteState {
    Unquoted,
    SingleQuoted,
    DoubleQuoted,
    DollarSingleQuoted,
}

/// Tokenize a single input line according to POSIX §2.3 + §2.7.
///
/// Here-document bodies are NOT read here — the REPL loop must detect
/// `Token::HereDoc` tokens and call `read_heredoc_body` for each one before
/// executing the command.
pub fn tokenize(input: &str) -> Result<Vec<Token>, &'static str> {
    let mut tokens: Vec<Token> = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    let mut current_word: Option<String> = None;
    let mut quote_state = QuoteState::Unquoted;

    macro_rules! begin_word {
        () => { if current_word.is_none() { current_word = Some(String::new()); } };
    }
    macro_rules! push_char {
        ($c:expr) => {{ begin_word!(); current_word.as_mut().unwrap().push($c); }};
    }
    macro_rules! flush_word {
        () => {
            if let Some(word) = current_word.take() {
                tokens.push(Token::Word(word));
            }
        };
    }

    while index < len {
        let byte = bytes[index];

        match quote_state {
            // ---------------------------------------------------------------
            // Unquoted context
            // ---------------------------------------------------------------
            QuoteState::Unquoted => {
                match byte {
                    b' ' | b'\t' => { flush_word!(); index += 1; }

                    b'#' if current_word.is_none() => { break; }

                    b'\\' => {
                        index += 1;
                        if index >= len {
                            push_char!('\\');
                        } else if bytes[index] == b'\n' {
                            index += 1; // line continuation
                        } else {
                            begin_word!();
                            push_byte_as_char(&mut current_word, bytes[index]);
                            index += 1;
                        }
                    }

                    b'\'' => { begin_word!(); quote_state = QuoteState::SingleQuoted; index += 1; }

                    b'$' if index + 1 < len && bytes[index + 1] == b'\'' => {
                        begin_word!();
                        quote_state = QuoteState::DollarSingleQuoted;
                        index += 2;
                    }

                    b'"' => { begin_word!(); quote_state = QuoteState::DoubleQuoted; index += 1; }

                    b'$' | b'`' => {
                        begin_word!();
                        push_byte_as_char(&mut current_word, byte);
                        index += 1;
                    }

                    // -------------------------------------------------------
                    // Digits: may be `[n]redir-op` (§2.7).
                    // Only treat as fd prefix if immediately followed by a
                    // redirect operator with no intervening whitespace.
                    // -------------------------------------------------------
                    b'0'..=b'9' => {
                        // Peek ahead: read all consecutive digits, then check
                        // if next char is a redirect operator start.
                        let digit_start = index;
                        let mut digit_end = index;
                        while digit_end < len && bytes[digit_end].is_ascii_digit() {
                            digit_end += 1;
                        }
                        let next_after_digits = if digit_end < len { bytes[digit_end] } else { 0 };
                        let is_redir_start = matches!(next_after_digits, b'<' | b'>');

                        if is_redir_start && current_word.is_none() {
                            // Parse the fd number and emit a redirect token.
                            let fd_str = core::str::from_utf8(&bytes[digit_start..digit_end])
                                .unwrap_or("0");
                            let fd: u32 = parse_u32(fd_str).unwrap_or(0);
                            flush_word!();
                            index = digit_end; // point at '<' or '>'
                            let (tok, consumed) = parse_redir_op(bytes, index, Some(fd));
                            tokens.push(tok);
                            index += consumed;
                        } else {
                            // Regular digit character — part of a word.
                            begin_word!();
                            push_byte_as_char(&mut current_word, byte);
                            index += 1;
                        }
                    }

                    // -------------------------------------------------------
                    // Redirect operators (no preceding fd digit)
                    // -------------------------------------------------------
                    b'<' | b'>' => {
                        flush_word!();
                        let (tok, consumed) = parse_redir_op(bytes, index, None);
                        tokens.push(tok);
                        index += consumed;
                    }

                    b'|' => {
                        flush_word!();
                        if index + 1 < len && bytes[index + 1] == b'|' {
                            tokens.push(Token::PipeOr);
                            index += 2;
                        } else {
                            tokens.push(Token::Pipe);
                            index += 1;
                        }
                    }

                    b'&' => {
                        flush_word!();
                        if index + 1 < len && bytes[index + 1] == b'&' {
                            tokens.push(Token::AmpAmp);
                            index += 2;
                        } else {
                            tokens.push(Token::Amp);
                            index += 1;
                        }
                    }

                    b';' => {
                        flush_word!();
                        if index + 1 < len && bytes[index + 1] == b';' {
                            tokens.push(Token::SemiSemi);
                            index += 2;
                        } else {
                            tokens.push(Token::Semi);
                            index += 1;
                        }
                    }

                    b'(' => { flush_word!(); tokens.push(Token::LParen);  index += 1; }
                    b')' => { flush_word!(); tokens.push(Token::RParen);  index += 1; }
                    b'\n' => { flush_word!(); tokens.push(Token::Newline); index += 1; }

                    _ => {
                        begin_word!();
                        push_byte_as_char(&mut current_word, byte);
                        index += 1;
                    }
                }
            }

            // ---------------------------------------------------------------
            // Single-quoted (§2.2.2)
            // ---------------------------------------------------------------
            QuoteState::SingleQuoted => {
                if byte == b'\'' { quote_state = QuoteState::Unquoted; index += 1; }
                else             { push_byte_as_char(&mut current_word, byte); index += 1; }
            }

            // ---------------------------------------------------------------
            // Double-quoted (§2.2.3)
            // ---------------------------------------------------------------
            QuoteState::DoubleQuoted => {
                match byte {
                    b'"' => { quote_state = QuoteState::Unquoted; index += 1; }
                    b'\\' => {
                        index += 1;
                        if index < len {
                            let next = bytes[index];
                            if matches!(next, b'$' | b'`' | b'\\' | b'\n' | b'"') {
                                if next == b'\n' { index += 1; }
                                else { push_byte_as_char(&mut current_word, next); index += 1; }
                            } else {
                                push_char!('\\');
                                push_byte_as_char(&mut current_word, next);
                                index += 1;
                            }
                        } else { push_char!('\\'); }
                    }
                    _ => { push_byte_as_char(&mut current_word, byte); index += 1; }
                }
            }

            // ---------------------------------------------------------------
            // Dollar-single-quoted (§2.2.4)
            // ---------------------------------------------------------------
            QuoteState::DollarSingleQuoted => {
                if byte == b'\'' {
                    quote_state = QuoteState::Unquoted;
                    index += 1;
                } else if byte == b'\\' {
                    index += 1;
                    if index >= len {
                        push_char!('\\');
                    } else {
                        let (ch, consumed) = parse_dollar_single_escape(bytes, index);
                        if let Some(c) = ch {
                            begin_word!();
                            current_word.as_mut().unwrap().push(c);
                        }
                        index += consumed;
                    }
                } else {
                    push_byte_as_char(&mut current_word, byte);
                    index += 1;
                }
            }
        }
    }

    match quote_state {
        QuoteState::SingleQuoted       => return Err("unterminated single-quote"),
        QuoteState::DoubleQuoted       => return Err("unterminated double-quote"),
        QuoteState::DollarSingleQuoted => return Err("unterminated $'...' quote"),
        QuoteState::Unquoted           => {}
    }
    flush_word!();
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// §2.7 Redirect operator parser
// ---------------------------------------------------------------------------

/// Parse a redirect operator starting at `bytes[index]`.
/// `fd` is the explicit fd number if one was parsed (e.g. `2` in `2>`).
/// Returns `(Token, bytes_consumed)`.
fn parse_redir_op(bytes: &[u8], index: usize, fd: Option<u32>) -> (Token, usize) {
    let len = bytes.len();
    let b0 = bytes[index];
    let b1 = if index + 1 < len { bytes[index + 1] } else { 0 };
    let b2 = if index + 2 < len { bytes[index + 2] } else { 0 };

    match (b0, b1, b2) {
        // <<- strip-tabs here-document
        (b'<', b'<', b'-') => (Token::HereDoc(fd, true),  3),
        // << here-document
        (b'<', b'<', _)    => (Token::HereDoc(fd, false), 2),
        // <& dup input
        (b'<', b'&', _)    => (Token::RedirDupIn(fd),     2),
        // <> read/write
        (b'<', b'>', _)    => (Token::RedirReadWrite(fd),  2),
        // < input
        (b'<', _, _)       => (Token::RedirIn(fd),         1),
        // >| no-clobber
        (b'>', b'|', _)    => (Token::RedirOutNoclobber(fd), 2),
        // >> append
        (b'>', b'>', _)    => (Token::RedirAppend(fd),     2),
        // >& dup output
        (b'>', b'&', _)    => (Token::RedirDupOut(fd),     2),
        // > output
        (b'>', _, _)       => (Token::RedirOut(fd),        1),
        _                  => (Token::RedirOut(fd),        1), // unreachable
    }
}

// ---------------------------------------------------------------------------
// §2.7.4 Here-document body reading
// ---------------------------------------------------------------------------

/// Read lines from `read_line_fn` until the delimiter line is found.
///
/// Called by the REPL loop after tokenizing a line that contains a
/// `Token::HereDoc` operator. The delimiter is the word that follows the
/// `<<` in the token stream (already extracted by the parser).
///
/// `quoted_delimiter`: if the original delimiter word was quoted (any part),
/// no expansions are performed on the body lines (§2.7.4).
/// `strip_tabs`: if `<<-`, leading tabs are stripped from each line.
///
/// Returns the here-document body as a single string (with `\n` line endings).
pub fn read_heredoc_body<F>(
    delimiter: &str,
    quoted_delimiter: bool,
    strip_tabs: bool,
    mut read_line_fn: F,
    ps2: &str,
) -> String
where
    F: FnMut() -> Option<String>,
{
    let mut body = String::new();

    loop {
        // §2.7.4: if interactive, write PS2 before each line.
        if !ps2.is_empty() {
            crate::write_err(ps2);
        }

        let line = match read_line_fn() {
            Some(l) => l,
            None    => break, // EOF — treat as end of here-document
        };

        let line_to_check = if strip_tabs {
            line.trim_start_matches('\t').to_string()
        } else {
            line.clone()
        };

        // Terminating delimiter line: only the delimiter text, nothing else.
        if line_to_check == delimiter {
            break;
        }

        // §2.7.4: <<- strips leading tabs from content lines too.
        let content_line = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };

        body.push_str(content_line);
        body.push('\n');
    }

    body
}

// ---------------------------------------------------------------------------
// Dollar-single-quote escape parser (§2.2.4)
// ---------------------------------------------------------------------------

fn parse_dollar_single_escape(bytes: &[u8], index: usize) -> (Option<char>, usize) {
    if index >= bytes.len() { return (Some('\\'), 0); }
    match bytes[index] {
        b'\'' => (Some('\''), 1),
        b'"'  => (Some('"'),  1),
        b'\\' => (Some('\\'), 1),
        b'a'  => (Some('\x07'), 1),
        b'b'  => (Some('\x08'), 1),
        b'e'  => (Some('\x1B'), 1),
        b'f'  => (Some('\x0C'), 1),
        b'n'  => (Some('\n'),   1),
        b'r'  => (Some('\r'),   1),
        b't'  => (Some('\t'),   1),
        b'v'  => (Some('\x0B'), 1),
        b'x' => {
            let mut value: u8 = 0;
            let mut consumed = 1usize;
            let mut digits = 0usize;
            while digits < 2 && index + consumed < bytes.len() {
                let b = bytes[index + consumed];
                if let Some(v) = hex_digit(b) {
                    value = value.wrapping_mul(16).wrapping_add(v);
                    consumed += 1; digits += 1;
                } else { break; }
            }
            if digits == 0 { (Some('x'), 1) }
            else if value == 0 { (None, consumed) }
            else { (Some(value as char), consumed) }
        }
        c if c >= b'0' && c <= b'7' => {
            let mut value: u32 = 0;
            let mut consumed = 0usize;
            while consumed < 3 && index + consumed < bytes.len() {
                let d = bytes[index + consumed];
                if d >= b'0' && d <= b'7' { value = value * 8 + (d - b'0') as u32; consumed += 1; }
                else { break; }
            }
            if value == 0 { (None, consumed) }
            else if value > 0xFF { (None, consumed) }
            else { (Some(value as u8 as char), consumed) }
        }
        _c => (Some('\\'), 0),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _           => None,
    }
}

#[inline]
fn push_byte_as_char(word: &mut Option<String>, byte: u8) {
    if let Some(w) = word { w.push(byte as char); }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for c in s.chars() {
        let d = c.to_digit(10)?;
        result = result.checked_mul(10)?.checked_add(d)?;
    }
    Some(result)
}
