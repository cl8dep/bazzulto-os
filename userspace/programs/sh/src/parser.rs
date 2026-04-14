// sh/parser.rs — POSIX §2.9 Shell Commands
//
// Spec: https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/sh.html
//
// Implemented:
//   §2.9.1   Simple Commands (assignments, words, redirects)
//   §2.9.1.1 Order of Processing
//   §2.9.1.2 Variable Assignments
//   §2.9.1.3 No Command Name
//   §2.9.2   Pipelines: [!] cmd1 [| cmd2 ...]  — negate flag
//   §2.9.3   Lists: cmd1 && cmd2, cmd1 || cmd2, cmd1 ; cmd2, cmd & (background)
//   §2.9.4.1 Grouping Commands: ( compound-list ) and { compound-list ; }
//   §2.9.4.2 The for Loop: for name [in words]; do compound-list; done
//   §2.9.4.3 The case Conditional: case word in [(pattern|...) compound-list;;] ... esac
//   §2.9.4.4 The if Conditional: if cond; then body; [elif cond; then body;] [else body;] fi
//   §2.9.4.5 The while Loop: while cond; do body; done
//   §2.9.4.6 The until Loop: until cond; do body; done
//
// The top-level parse result is a `List` (Vec<AndOrItem>) — a sequence of
// pipelines joined by `&&` / `||`, terminated by `;`, `&`, or end-of-input.
// Compound commands embed a nested `List` as their body.
//
// Multi-line compound commands are supported: the parser stops at EOF inside
// a compound command and signals `Err(NeedMore)` so the REPL can ask for
// another line.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::lexer::{Token, is_reserved_word};
use crate::vars::parse_assignment;

// ---------------------------------------------------------------------------
// AST nodes
// ---------------------------------------------------------------------------

/// A redirect operation on a simple command (§2.7).
///
/// The `fd` field carries the explicit file descriptor number when the
/// syntax `[n]redir-op` was used (e.g. `2>file` sets `fd = Some(2)`).
/// When `fd` is `None` the default for that operator applies:
///   stdin redirects default to fd 0, stdout redirects to fd 1.
#[derive(Debug, Clone)]
pub enum Redirect {
    /// `[n]< filename` — open filename as input (§2.7.1). Default fd 0.
    StdinFrom(Option<u32>, String),
    /// `[n]> filename` — open/create filename as output, truncate (§2.7.2). Default fd 1.
    StdoutTo(Option<u32>, String),
    /// `[n]>> filename` — open/create filename as output, append (§2.7.3). Default fd 1.
    StdoutAppend(Option<u32>, String),
    /// `[n]>| filename` — same as StdoutTo; noclobber override (§2.7.2). Default fd 1.
    StdoutNoclobber(Option<u32>, String),
    /// `[n]>& word` — duplicate output fd (§2.7.6). Default fd 1.
    DupOut(Option<u32>, String),
    /// `[n]<& word` — duplicate input fd (§2.7.6). Default fd 0.
    DupIn(Option<u32>, String),
    /// `[n]<> filename` — open file read/write (§2.7.7). Default fd 0.
    ReadWrite(Option<u32>, String),
    /// `[n]<< word` — here-document (§2.7.4). `strip_tabs` = `<<-`. Default fd 0.
    HereDoc(Option<u32>, bool, String),
}

/// A simple command: optional leading variable assignments, words, and redirects (§2.9.1).
#[derive(Debug, Clone)]
pub struct SimpleCommand {
    /// Leading `NAME=value` assignments (§2.9.1.1 step 1).
    pub assignments: Vec<String>,
    /// argv[0..n]: command name followed by arguments (may be empty — §2.9.1.3).
    pub words: Vec<String>,
    /// Redirect operations associated with this command.
    pub redirects: Vec<Redirect>,
}

/// A pipeline: [!] cmd1 [| cmd2 ...] (§2.9.2).
///
/// `negate`: if true, the exit status of the pipeline is logically negated
/// (0 becomes 1, non-zero becomes 0) per the `!` reserved word.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Whether the exit status is negated (§2.9.2 `!` prefix).
    pub negate: bool,
    pub commands: Vec<SimpleCommand>,
}

/// A compound command body — a list of `AndOrItem`s.
pub type CompoundList = Vec<AndOrItem>;

/// One item in a list — a pipeline decorated with the operator that
/// connects it to the *next* item.
#[derive(Debug, Clone)]
pub struct AndOrItem {
    /// The pipeline (or compound command embedded as a single-stage pipeline).
    pub pipeline: Pipeline,
    /// How this item connects to the *next* item.
    /// `None` for the last item in the list (or when terminated by `;` / newline).
    pub separator: Separator,
}

/// The separator that follows a pipeline in a list.
#[derive(Debug, Clone, PartialEq)]
pub enum Separator {
    /// `;` or newline — synchronous sequential execution.
    Semi,
    /// `&` — asynchronous execution (background).
    Amp,
    /// `&&` — AND list: execute next only if this succeeded.
    And,
    /// `||` — OR list: execute next only if this failed.
    Or,
    /// End of input — no explicit separator.
    End,
}

// ---------------------------------------------------------------------------
// Parse error
// ---------------------------------------------------------------------------

/// A parse error.
///
/// `NeedMore` is returned when a compound command is not yet complete — the
/// caller should read another line, append it to the token stream, and retry.
#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// Hard syntax error: message describes the problem.
    Syntax(&'static str),
    /// The input was incomplete (e.g. `for … do` body not closed by `done`).
    NeedMore,
}

impl ParseError {
    pub fn message(&self) -> &'static str {
        match self {
            ParseError::Syntax(m) => m,
            ParseError::NeedMore  => "unexpected end of input",
        }
    }
}

// ---------------------------------------------------------------------------
// Parser state
// ---------------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Token],
    index:  usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Parser { tokens, index: 0 }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.index)
    }

    fn advance(&mut self) -> Option<&'a Token> {
        let tok = self.tokens.get(self.index);
        if tok.is_some() { self.index += 1; }
        tok
    }

    fn is_at_end(&self) -> bool {
        self.index >= self.tokens.len()
    }

    /// Skip zero or more newlines (used inside compound commands per §2.9.4).
    fn skip_newlines(&mut self) {
        while let Some(Token::Newline) = self.peek() {
            self.index += 1;
        }
    }

    /// Check if the next token is the reserved word `word` (§2.4: only in
    /// command-name / reserved-word position).
    fn peek_reserved(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w.as_str() == word)
    }

    /// Check if the next token is `}`.
    fn peek_rbrace(&self) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w.as_str() == "}")
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Parse a flat token list into a `CompoundList` (a sequence of `AndOrItem`s).
///
/// Returns `Err(ParseError::NeedMore)` when the input ends inside a compound
/// command that is not yet complete (signals REPL to request another line).
///
/// Returns `Err(ParseError::Syntax(...))` on hard parse errors.
pub fn parse_compound_list(tokens: &[Token]) -> Result<CompoundList, ParseError> {
    let mut parser = Parser::new(tokens);
    let list = parse_list(&mut parser, ListContext::TopLevel)?;
    Ok(list)
}

// ---------------------------------------------------------------------------
// List context
// ---------------------------------------------------------------------------

/// Context in which a list is being parsed — controls what terminates it.
#[derive(Clone, Copy, PartialEq)]
enum ListContext {
    /// Top-level REPL input; terminated by end-of-tokens.
    TopLevel,
    /// Inside `do … done`.
    DoDone,
    /// Inside `( … )`.
    Subshell,
    /// Inside `{ … }`.
    Group,
    /// Inside `if … then` or `elif … then` condition list: terminated by `then`.
    IfThen,
    /// Inside `then … elif|else|fi` or `else … fi` body: terminated by `elif`, `else`, or `fi`.
    IfBody,
    /// Inside `else … fi` body: terminated by `fi`.
    ElseBody,
    /// Inside `case … in` subject (just one word consumed by parse_case directly).
    /// Terminated by the first pattern-clause start or `esac`.
    CaseBody,
}

// ---------------------------------------------------------------------------
// §2.9.3 List parsing
// ---------------------------------------------------------------------------

/// Parse a list of AND-OR items (§2.9.3).
///
/// A list is terminated by:
///   - End of token stream (TopLevel / always acceptable)
///   - `done` keyword (DoDone context)
///   - `)` token   (Subshell context)
///   - `}` word    (Group context)
fn parse_list(parser: &mut Parser, ctx: ListContext) -> Result<CompoundList, ParseError> {
    let mut list: CompoundList = Vec::new();

    loop {
        parser.skip_newlines();

        // Check termination conditions.
        let done = match ctx {
            ListContext::DoDone   => parser.peek_reserved("done"),
            ListContext::Subshell => matches!(parser.peek(), Some(Token::RParen)),
            ListContext::Group    => parser.peek_rbrace(),
            ListContext::TopLevel => parser.is_at_end(),
            // `if … then`: terminate on `then`
            ListContext::IfThen   => parser.peek_reserved("then"),
            // `then … fi/elif/else`: terminate on any of those
            ListContext::IfBody   => parser.peek_reserved("elif")
                                  || parser.peek_reserved("else")
                                  || parser.peek_reserved("fi"),
            // `else … fi`: terminate on `fi`
            ListContext::ElseBody => parser.peek_reserved("fi"),
            // case body handled inline in parse_case, not via parse_list
            ListContext::CaseBody => parser.peek_reserved("esac"),
        };
        if done { break; }

        if parser.is_at_end() {
            // End of tokens inside a compound command → need more input.
            if ctx != ListContext::TopLevel {
                return Err(ParseError::NeedMore);
            }
            break;
        }

        // Parse one pipeline (or compound command).
        let (pipeline, separator) = parse_and_or(parser, ctx)?;

        list.push(AndOrItem { pipeline, separator: separator.clone() });

        // After `&&` / `||` continue parsing another pipeline in the same list.
        // After `;` / `&` / newline / end we end the current item and loop for the next.
        match separator {
            Separator::And | Separator::Or => { /* loop — next pipeline is part of this AND/OR chain */ }
            Separator::Semi | Separator::Amp | Separator::End => {
                // End of this list item; check if there are more.
                // If context expects more (DoDone / Subshell / Group) loop again.
                match ctx {
                    ListContext::TopLevel => break, // one statement per REPL line
                    _ => { /* loop */ }
                }
            }
        }
    }

    Ok(list)
}

/// Parse one pipeline and its following separator operator (§2.9.3).
///
/// Returns `(pipeline, separator)`.
fn parse_and_or(parser: &mut Parser, _ctx: ListContext) -> Result<(Pipeline, Separator), ParseError> {
    let pipeline = parse_pipeline(parser)?;

    let separator = match parser.peek() {
        Some(Token::AmpAmp) => { parser.advance(); Separator::And }
        Some(Token::PipeOr) => { parser.advance(); Separator::Or  }
        Some(Token::Amp)    => { parser.advance(); Separator::Amp }
        Some(Token::Semi) | Some(Token::Newline) => {
            parser.advance();
            Separator::Semi
        }
        _ => Separator::End,
    };

    Ok((pipeline, separator))
}

// ---------------------------------------------------------------------------
// §2.9.2 Pipeline
// ---------------------------------------------------------------------------

/// Parse a pipeline: `[!] command [| command ...]` (§2.9.2).
fn parse_pipeline(parser: &mut Parser) -> Result<Pipeline, ParseError> {
    // §2.9.2: `!` negates the exit status of the whole pipeline.
    let negate = if parser.peek_reserved("!") {
        parser.advance();
        true
    } else {
        false
    };

    let mut commands: Vec<SimpleCommand> = Vec::new();
    commands.push(parse_simple_or_compound(parser)?);

    while matches!(parser.peek(), Some(Token::Pipe)) {
        parser.advance(); // consume `|`
        parser.skip_newlines(); // §2.9.2: newlines allowed after `|`
        commands.push(parse_simple_or_compound(parser)?);
    }

    Ok(Pipeline { negate, commands })
}

// ---------------------------------------------------------------------------
// §2.9.4 Compound commands vs simple commands
// ---------------------------------------------------------------------------

/// Parse either a simple command or a compound command, returning it as a
/// `SimpleCommand` (compound commands are wrapped in a synthetic shell call).
///
/// Because `Pipeline.commands` is `Vec<SimpleCommand>`, compound commands that
/// appear as a pipeline stage are represented as a `SimpleCommand` with a
/// special internal marker — see `CompoundMarker` below.
///
/// Actually the cleaner design is to add a `Command` variant to the pipeline.
/// We will embed the compound command in a `Command` wrapper here and store it
/// via a `CompoundCommand` variant in `SimpleCommand`.  But since we want to
/// keep the executor interface uniform, we instead parse compound commands
/// at the `parse_pipeline` level and box them into a one-element pipeline.
///
/// Architectural choice for this implementation: `parse_pipeline` delegates to
/// this function for each pipeline stage. If a compound command is encountered
/// it is parsed and wrapped in a special `SimpleCommand` using the
/// `COMPOUND_TAG` prefix in `words[0]` — the executor pattern-matches on this
/// to dispatch correctly. This is simpler than a full recursive ADT under the
/// `Pipeline` type.
///
/// Tag values (not valid POSIX command names — contain NUL byte):
///   `\x00subshell` / `\x00group` / `\x00for`
///
/// The serialized compound-command data is appended as subsequent words.
/// The executor deserializes them back.  This keeps the type system unchanged
/// while supporting the full compound-command grammar.
fn parse_simple_or_compound(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    match parser.peek() {
        Some(Token::LParen) => parse_subshell(parser),
        Some(Token::Word(w)) if w.as_str() == "{"     => parse_group(parser),
        Some(Token::Word(w)) if w.as_str() == "for"   => parse_for(parser),
        Some(Token::Word(w)) if w.as_str() == "if"    => parse_if(parser),
        Some(Token::Word(w)) if w.as_str() == "while" => parse_while(parser, false),
        Some(Token::Word(w)) if w.as_str() == "until" => parse_while(parser, true),
        Some(Token::Word(w)) if w.as_str() == "case"  => parse_case(parser),
        // §2.9.5 function definition: fname ( ) compound-command
        // Detect: Token::Word (valid name) followed by `(` `)`.
        Some(Token::Word(_)) if is_funcdef(parser)    => parse_funcdef(parser),
        _ => parse_simple_command(parser),
    }
}

/// Peek ahead two tokens to detect the `name ( )` function definition prefix.
fn is_funcdef(parser: &Parser) -> bool {
    // tokens[index] is Word(name), tokens[index+1] must be LParen, then RParen.
    // We need to skip any intervening Newline tokens between ( and ) per POSIX.
    if parser.index + 2 > parser.tokens.len() {
        return false;
    }
    // tokens[index+1] must be LParen
    if !matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen)) {
        return false;
    }
    // tokens[index+2] may have newlines then RParen
    let mut j = parser.index + 2;
    while matches!(parser.tokens.get(j), Some(Token::Newline)) {
        j += 1;
    }
    matches!(parser.tokens.get(j), Some(Token::RParen))
}

// ---------------------------------------------------------------------------
// §2.9.4.1 Subshell: ( compound-list )
// ---------------------------------------------------------------------------

fn parse_subshell(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `(`
    parser.skip_newlines();

    let body = parse_list(parser, ListContext::Subshell)?;

    // Expect `)`.
    match parser.peek() {
        Some(Token::RParen) => { parser.advance(); }
        None => return Err(ParseError::NeedMore),
        _ => return Err(ParseError::Syntax("expected ')' to close subshell")),
    }

    Ok(make_compound_simple("subshell", &body))
}

// ---------------------------------------------------------------------------
// §2.9.4.1 Group: { compound-list ; }
// ---------------------------------------------------------------------------

fn parse_group(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `{`
    parser.skip_newlines();

    let body = parse_list(parser, ListContext::Group)?;

    // Expect `}`.
    match parser.peek() {
        Some(Token::Word(w)) if w.as_str() == "}" => { parser.advance(); }
        None => return Err(ParseError::NeedMore),
        _ => return Err(ParseError::Syntax("expected '}' to close group")),
    }

    Ok(make_compound_simple("group", &body))
}

// ---------------------------------------------------------------------------
// §2.9.4.2 For loop: for name [in word ...] [;] do compound-list done
// ---------------------------------------------------------------------------

fn parse_for(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `for`

    // Variable name.
    let variable = match parser.advance() {
        Some(Token::Word(name)) => {
            if is_reserved_word(name.as_str()) {
                return Err(ParseError::Syntax("'for': reserved word as variable name"));
            }
            name.clone()
        }
        None => return Err(ParseError::NeedMore),
        _ => return Err(ParseError::Syntax("'for': expected variable name")),
    };

    parser.skip_newlines();

    // Optional `in wordlist`.
    let words: Option<Vec<String>> = if parser.peek_reserved("in") {
        parser.advance(); // consume `in`
        let mut word_list: Vec<String> = Vec::new();
        loop {
            match parser.peek() {
                Some(Token::Word(w)) => {
                    word_list.push(w.clone());
                    parser.advance();
                }
                Some(Token::Semi) | Some(Token::Newline) => {
                    parser.advance();
                    break;
                }
                _ => break,
            }
        }
        Some(word_list)
    } else {
        // No `in` clause — consume optional `;` or newline.
        match parser.peek() {
            Some(Token::Semi) | Some(Token::Newline) => { parser.advance(); }
            _ => {}
        }
        None
    };

    parser.skip_newlines();

    // `do`
    if !parser.peek_reserved("do") {
        if parser.is_at_end() { return Err(ParseError::NeedMore); }
        return Err(ParseError::Syntax("'for': expected 'do'"));
    }
    parser.advance(); // consume `do`
    parser.skip_newlines();

    // Body — terminated by `done`.
    let body = parse_list(parser, ListContext::DoDone)?;

    // `done`
    if !parser.peek_reserved("done") {
        if parser.is_at_end() { return Err(ParseError::NeedMore); }
        return Err(ParseError::Syntax("'for': expected 'done'"));
    }
    parser.advance(); // consume `done`

    Ok(make_for_simple(&variable, &words, &body))
}

// ---------------------------------------------------------------------------
// §2.9.4.4 If conditional: if cond; then body; [elif cond; then body;] [else body;] fi
// ---------------------------------------------------------------------------

fn parse_if(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `if`

    // Condition list: terminated by `then`.
    let condition = parse_list(parser, ListContext::IfThen)?;

    if !parser.peek_reserved("then") {
        return if parser.is_at_end() { Err(ParseError::NeedMore) }
               else { Err(ParseError::Syntax("'if': expected 'then'")) };
    }
    parser.advance(); // consume `then`
    parser.skip_newlines();

    // Then-body: terminated by `elif`, `else`, or `fi`.
    let then_body = parse_list(parser, ListContext::IfBody)?;

    // Collect elif/else clauses.
    // Serialization: TAG_IF <cond> TAG_THEN <body>
    //                [TAG_ELIF <cond> TAG_THEN <body>] ...
    //                [TAG_ELSE <body>]
    //                TAG_FI
    let mut words: Vec<String> = Vec::new();
    words.push(TAG_IF.into());
    serialize_list(&condition, &mut words);
    words.push(TAG_THEN.into());
    serialize_list(&then_body, &mut words);

    // Zero or more elif clauses.
    while parser.peek_reserved("elif") {
        parser.advance(); // consume `elif`
        parser.skip_newlines();
        let elif_cond = parse_list(parser, ListContext::IfThen)?;
        if !parser.peek_reserved("then") {
            return if parser.is_at_end() { Err(ParseError::NeedMore) }
                   else { Err(ParseError::Syntax("'elif': expected 'then'")) };
        }
        parser.advance(); // consume `then`
        parser.skip_newlines();
        let elif_body = parse_list(parser, ListContext::IfBody)?;
        words.push(TAG_ELIF.into());
        serialize_list(&elif_cond, &mut words);
        words.push(TAG_THEN.into());
        serialize_list(&elif_body, &mut words);
    }

    // Optional else clause.
    if parser.peek_reserved("else") {
        parser.advance(); // consume `else`
        parser.skip_newlines();
        let else_body = parse_list(parser, ListContext::ElseBody)?;
        words.push(TAG_ELSE.into());
        serialize_list(&else_body, &mut words);
    }

    // `fi`
    if !parser.peek_reserved("fi") {
        return if parser.is_at_end() { Err(ParseError::NeedMore) }
               else { Err(ParseError::Syntax("'if': expected 'fi'")) };
    }
    parser.advance(); // consume `fi`
    words.push(TAG_FI.into());

    Ok(SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() })
}

// ---------------------------------------------------------------------------
// §2.9.4.5/.6 While/Until loop: while cond; do body; done
// ---------------------------------------------------------------------------

fn parse_while(parser: &mut Parser, is_until: bool) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `while` or `until`

    // Condition list: terminated by `do` (not `done`).
    // Parse items one at a time, stopping when `do` is at statement-start position.
    let mut cond_list: CompoundList = Vec::new();
    loop {
        parser.skip_newlines();
        if parser.peek_reserved("do") { break; }
        if parser.is_at_end() { return Err(ParseError::NeedMore); }
        let (pipeline, separator) = parse_and_or(parser, ListContext::TopLevel)?;
        let is_end = matches!(separator, Separator::End);
        cond_list.push(AndOrItem { pipeline, separator });
        // After a `;;`-less end, loop to check for `do` on the next line.
        if is_end { /* loop — check for `do` at top */ }
    }

    if !parser.peek_reserved("do") {
        return if parser.is_at_end() { Err(ParseError::NeedMore) }
               else { Err(ParseError::Syntax("'while'/'until': expected 'do'")) };
    }
    parser.advance(); // consume `do`
    parser.skip_newlines();

    // Body: terminated by `done`.
    let body = parse_list(parser, ListContext::DoDone)?;

    if !parser.peek_reserved("done") {
        return if parser.is_at_end() { Err(ParseError::NeedMore) }
               else { Err(ParseError::Syntax("'while'/'until': expected 'done'")) };
    }
    parser.advance(); // consume `done`

    let tag = if is_until { TAG_UNTIL } else { TAG_WHILE };
    let mut words: Vec<String> = Vec::new();
    words.push(tag.into());
    serialize_list(&cond_list, &mut words);
    words.push(TAG_BODY.into());
    serialize_list(&body, &mut words);

    Ok(SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() })
}

// ---------------------------------------------------------------------------
// §2.9.4.3 Case conditional: case word in [pattern [| pattern ...]) list ;;] ... esac
// ---------------------------------------------------------------------------

fn parse_case(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    parser.advance(); // consume `case`
    parser.skip_newlines();

    // Subject word.
    let subject = match parser.peek() {
        Some(Token::Word(w)) => { let w = w.clone(); parser.advance(); w }
        None => return Err(ParseError::NeedMore),
        _ => return Err(ParseError::Syntax("'case': expected word")),
    };

    parser.skip_newlines();

    // `in`
    if !parser.peek_reserved("in") {
        return if parser.is_at_end() { Err(ParseError::NeedMore) }
               else { Err(ParseError::Syntax("'case': expected 'in'")) };
    }
    parser.advance(); // consume `in`
    parser.skip_newlines();

    // Serialization:
    //   words[0] = TAG_CASE
    //   words[1] = subject
    //   For each clause:
    //     TAG_CASE_ITEM
    //     <N: pattern count>
    //     <pattern1> ... <patternN>
    //     TAG_BODY
    //     <serialized body list>
    //     TAG_CASE_SEP  (";;" | ";&" | ";;&")
    //   TAG_ESAC
    let mut words: Vec<String> = Vec::new();
    words.push(TAG_CASE.into());
    words.push(subject);

    // Parse zero or more pattern clauses.
    loop {
        parser.skip_newlines();

        // `esac` ends the case.
        if parser.peek_reserved("esac") {
            parser.advance();
            break;
        }

        if parser.is_at_end() {
            return Err(ParseError::NeedMore);
        }

        // Optional `(` before the pattern list.
        if matches!(parser.peek(), Some(Token::LParen)) {
            parser.advance();
        }

        // One or more patterns separated by `|`.
        let mut patterns: Vec<String> = Vec::new();
        loop {
            match parser.peek() {
                Some(Token::Word(w)) => {
                    patterns.push(w.clone());
                    parser.advance();
                }
                _ => return Err(ParseError::Syntax("'case': expected pattern")),
            }
            match parser.peek() {
                Some(Token::Pipe) => { parser.advance(); } // `|` separates patterns
                Some(Token::RParen) => { parser.advance(); break; } // `)` ends pattern list
                None => return Err(ParseError::NeedMore),
                _ => return Err(ParseError::Syntax("'case': expected ')' after pattern")),
            }
        }

        parser.skip_newlines();

        // Body: terminated by `;;`, `;&`, `;;&`, or `esac`.
        // We parse items until we hit `;;` / `;&` / `;;&` / `esac`.
        let body = parse_case_body(parser)?;

        // Terminator.
        let sep = match parser.peek() {
            Some(Token::SemiSemi) => { parser.advance(); ";;" }
            Some(Token::Word(w)) if w.as_str() == ";&"  => { parser.advance(); ";&" }
            Some(Token::Word(w)) if w.as_str() == ";;&" => { parser.advance(); ";;&" }
            // If we hit `esac` after the body: no explicit `;;` — treat as `;;`.
            Some(Token::Word(w)) if w.as_str() == "esac" => { ";;" }
            None => return Err(ParseError::NeedMore),
            _ => return Err(ParseError::Syntax("'case': expected ';;', ';&', ';;&', or 'esac'")),
        };

        words.push(TAG_CASE_ITEM.into());
        words.push(alloc::format!("{}", patterns.len()));
        for p in &patterns { words.push(p.clone()); }
        words.push(TAG_BODY.into());
        serialize_list(&body, &mut words);
        words.push(sep.into());
    }

    words.push(TAG_ESAC.into());
    Ok(SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() })
}

/// Parse a case-clause body: items terminated by `;;`, `;&`, `;;&`, or `esac`.
fn parse_case_body(parser: &mut Parser) -> Result<CompoundList, ParseError> {
    let mut list: CompoundList = Vec::new();
    loop {
        parser.skip_newlines();
        // Stop on `;;`, `;&`, `;;&`, `esac`, or end of input.
        match parser.peek() {
            Some(Token::SemiSemi) => break,
            Some(Token::Word(w)) if matches!(w.as_str(), ";&" | ";;&" | "esac") => break,
            None => return Err(ParseError::NeedMore),
            _ => {}
        }
        let (pipeline, separator) = parse_and_or(parser, ListContext::TopLevel)?;
        let is_final = matches!(separator, Separator::End);
        list.push(AndOrItem { pipeline, separator });
        if is_final { break; }
    }
    Ok(list)
}

// ---------------------------------------------------------------------------
// §2.9.5 Function definition: fname () compound-command
// ---------------------------------------------------------------------------
//
// Serialized format:
//   words[0] = TAG_FUNCDEF
//   words[1] = function name
//   words[2..] = body compound-command serialized (TAG_SUBSHELL / TAG_GROUP / etc.)
//
// The body is a single compound command (subshell, group, for, while, etc.).
// POSIX §2.9.5 requires a compound command as the function body.

fn parse_funcdef(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    // Consume function name.
    let name = match parser.advance() {
        Some(Token::Word(w)) => w.clone(),
        _ => return Err(ParseError::Syntax("expected function name")),
    };

    // Consume `(`.
    match parser.advance() {
        Some(Token::LParen) => {}
        _ => return Err(ParseError::Syntax("expected '(' in function definition")),
    }

    // Skip optional newlines before `)`.
    parser.skip_newlines();

    // Consume `)`.
    match parser.advance() {
        Some(Token::RParen) => {}
        _ => return Err(ParseError::Syntax("expected ')' in function definition")),
    }

    // Skip newlines before the body compound command.
    parser.skip_newlines();

    // Parse the body: must be a compound command (group `{ }`, subshell `( )`,
    // for, if, while, until, case).
    let body_cmd = match parser.peek() {
        Some(Token::LParen)
        | Some(Token::Word(_)) => parse_simple_or_compound(parser)?,
        None => return Err(ParseError::NeedMore),
        _ => return Err(ParseError::Syntax("expected compound command as function body")),
    };

    // Build serialized words: TAG_FUNCDEF name body_cmd_words...
    let mut words: Vec<String> = Vec::new();
    words.push(TAG_FUNCDEF.into());
    words.push(name);
    words.extend(body_cmd.words);

    Ok(SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() })
}

// ---------------------------------------------------------------------------
// §2.9.1 Simple command
// ---------------------------------------------------------------------------

fn parse_simple_command(parser: &mut Parser) -> Result<SimpleCommand, ParseError> {
    let mut assignments: Vec<String> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut redirects: Vec<Redirect> = Vec::new();
    let mut in_assignment_position = true;

    loop {
        match parser.peek() {
            None
            | Some(Token::Semi)
            | Some(Token::Newline)
            | Some(Token::Amp)
            | Some(Token::AmpAmp)
            | Some(Token::PipeOr)
            | Some(Token::Pipe)
            | Some(Token::RParen)
            | Some(Token::SemiSemi) => break,

            Some(Token::Word(w)) => {
                // Check for compound-command reserved words when no words yet.
                if words.is_empty() && assignments.is_empty() {
                    let s = w.as_str();
                    if matches!(s, "}" | "do" | "done" | "fi" | "then" | "elif"
                                   | "else" | "esac" | "in") {
                        // These terminate a compound-command body — stop here.
                        break;
                    }
                }

                let w = w.clone();
                if in_assignment_position && parse_assignment(w.as_str()).is_some() {
                    assignments.push(w);
                } else {
                    in_assignment_position = false;
                    let _ = is_reserved_word(w.as_str()); // reserved word as command name — OK
                    words.push(w);
                }
                parser.advance();
            }

            Some(tok) if tok.is_redirect_op() => {
                in_assignment_position = false;
                let redirect = parse_redirect(parser)?;
                redirects.push(redirect);
            }

            Some(Token::LParen) | Some(Token::AmpAmp) | Some(Token::PipeOr) => break,

            _ => {
                parser.advance(); // skip unrecognized token
            }
        }
    }

    Ok(SimpleCommand { assignments, words, redirects })
}

// ---------------------------------------------------------------------------
// Redirect parsing (§2.7)
// ---------------------------------------------------------------------------

fn parse_redirect(parser: &mut Parser) -> Result<Redirect, ParseError> {
    let tok = parser.advance().ok_or(ParseError::Syntax("expected redirect operator"))?;

    macro_rules! next_word {
        ($msg:literal) => {{
            parser.peek().and_then(|t| t.as_word().map(|s| s.to_string()))
                .ok_or(ParseError::Syntax($msg))
                .map(|w| { parser.advance(); w })?
        }};
    }

    let redirect = match tok {
        Token::RedirIn(fd)           => Redirect::StdinFrom(*fd,         next_word!("expected filename after '<'")),
        Token::RedirOut(fd)          => Redirect::StdoutTo(*fd,          next_word!("expected filename after '>'")),
        Token::RedirAppend(fd)       => Redirect::StdoutAppend(*fd,      next_word!("expected filename after '>>'")),
        Token::RedirOutNoclobber(fd) => Redirect::StdoutNoclobber(*fd,   next_word!("expected filename after '>|'")),
        Token::RedirDupOut(fd)       => Redirect::DupOut(*fd,            next_word!("expected fd or '-' after '>&'")),
        Token::RedirDupIn(fd)        => Redirect::DupIn(*fd,             next_word!("expected fd or '-' after '<&'")),
        Token::RedirReadWrite(fd)    => Redirect::ReadWrite(*fd,         next_word!("expected filename after '<>'")),
        Token::HereDoc(fd, strip)    => Redirect::HereDoc(*fd, *strip,   next_word!("expected delimiter after '<<'")),
        _ => return Err(ParseError::Syntax("internal: parse_redirect called on non-redirect token")),
    };

    Ok(redirect)
}

// ---------------------------------------------------------------------------
// Compound-command serialization / deserialization
// ---------------------------------------------------------------------------
//
// Compound commands (subshell, group, for) are embedded in a `SimpleCommand`
// using a synthetic `words[0]` tag so the executor can identify them without
// changing the pipeline type.
//
// Serialization format:
//
//   Subshell / Group:
//     words[0] = "\x00subshell" | "\x00group"
//     words[1..] = JSON-like token stream (see `serialize_list`)
//
//   For:
//     words[0] = "\x00for"
//     words[1] = variable name
//     words[2] = "some" | "none"
//     words[3..] = word list items, then "\x00body", then serialized body
//
// The separator character \x00 cannot appear in a valid POSIX name or word
// (it terminates C strings) so it is safe as a sentinel.

pub const TAG_SUBSHELL:   &str = "\x00subshell";
pub const TAG_GROUP:      &str = "\x00group";
pub const TAG_FOR:        &str = "\x00for";
pub const TAG_IF:         &str = "\x00if";
pub const TAG_WHILE:      &str = "\x00while";
pub const TAG_UNTIL:      &str = "\x00until";
pub const TAG_CASE:       &str = "\x00case";
pub const TAG_BODY:       &str = "\x00body";
pub const TAG_THEN:       &str = "\x00then";
pub const TAG_ELIF:       &str = "\x00elif";
pub const TAG_ELSE:       &str = "\x00else";
pub const TAG_FI:         &str = "\x00fi";
pub const TAG_CASE_ITEM:  &str = "\x00case_item";
pub const TAG_ESAC:       &str = "\x00esac";
pub const TAG_AND:        &str = "\x00&&";
pub const TAG_OR:         &str = "\x00||";
pub const TAG_AMP:        &str = "\x00&";
pub const TAG_SEMI:       &str = "\x00;";
pub const TAG_END:        &str = "\x00end";
pub const TAG_NEGATE:     &str = "\x00!";
pub const TAG_CMD:        &str = "\x00cmd";
/// §2.9.5 Function definition: words[0]=TAG_FUNCDEF, words[1]=name, words[2..]=body.
pub const TAG_FUNCDEF:    &str = "\x00funcdef";

fn make_compound_simple(tag: &str, body: &CompoundList) -> SimpleCommand {
    let mut words: Vec<String> = Vec::new();
    words.push(alloc::format!("\x00{}", tag));
    serialize_list(body, &mut words);
    SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() }
}

fn make_for_simple(variable: &str, word_list: &Option<Vec<String>>, body: &CompoundList) -> SimpleCommand {
    let mut words: Vec<String> = Vec::new();
    words.push(TAG_FOR.into());
    words.push(variable.into());
    match word_list {
        None => words.push("none".into()),
        Some(wl) => {
            words.push(alloc::format!("some:{}", wl.len()));
            for w in wl { words.push(w.clone()); }
        }
    }
    words.push(TAG_BODY.into());
    serialize_list(body, &mut words);
    SimpleCommand { assignments: Vec::new(), words, redirects: Vec::new() }
}

/// Serialize a `CompoundList` into the words vector.
///
/// Format per `AndOrItem`:
///   TAG_CMD
///   <negate: "!" | "">
///   <n: number of SimpleCommands>
///   For each SimpleCommand:
///     <m: number of assignments> <assignment>...
///     <k: number of words> <word>...
///     <r: number of redirects> <redirect-tag> <args...>...
///   <separator-tag>
pub fn serialize_list(list: &CompoundList, out: &mut Vec<String>) {
    for item in list {
        out.push(TAG_CMD.into());
        out.push(if item.pipeline.negate { TAG_NEGATE.into() } else { String::new() });
        out.push(alloc::format!("{}", item.pipeline.commands.len()));
        for cmd in &item.pipeline.commands {
            // assignments
            out.push(alloc::format!("{}", cmd.assignments.len()));
            for a in &cmd.assignments { out.push(a.clone()); }
            // words
            out.push(alloc::format!("{}", cmd.words.len()));
            for w in &cmd.words { out.push(w.clone()); }
            // redirects
            out.push(alloc::format!("{}", cmd.redirects.len()));
            for r in &cmd.redirects {
                serialize_redirect(r, out);
            }
        }
        let sep_tag = match item.separator {
            Separator::And  => TAG_AND,
            Separator::Or   => TAG_OR,
            Separator::Amp  => TAG_AMP,
            Separator::Semi => TAG_SEMI,
            Separator::End  => TAG_END,
        };
        out.push(sep_tag.into());
    }
}

fn serialize_redirect(r: &Redirect, out: &mut Vec<String>) {
    // Format: <type-tag> <fd-or-none> <arg...>
    match r {
        Redirect::StdinFrom(fd, f)       => { out.push("StdinFrom".into());       push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::StdoutTo(fd, f)        => { out.push("StdoutTo".into());        push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::StdoutAppend(fd, f)    => { out.push("StdoutAppend".into());    push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::StdoutNoclobber(fd, f) => { out.push("StdoutNoclobber".into()); push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::DupOut(fd, f)          => { out.push("DupOut".into());          push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::DupIn(fd, f)           => { out.push("DupIn".into());           push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::ReadWrite(fd, f)       => { out.push("ReadWrite".into());       push_opt_fd(fd, out); out.push(f.clone()); }
        Redirect::HereDoc(fd, strip, f)  => {
            out.push("HereDoc".into());
            push_opt_fd(fd, out);
            out.push(if *strip { "1".into() } else { "0".into() });
            out.push(f.clone());
        }
    }
}

fn push_opt_fd(fd: &Option<u32>, out: &mut Vec<String>) {
    match fd {
        None    => out.push("_".into()),
        Some(n) => out.push(alloc::format!("{}", n)),
    }
}

// ---------------------------------------------------------------------------
// Deserialization — used by the executor
// ---------------------------------------------------------------------------

/// Deserialize a `CompoundList` from the synthetic words vector starting at `index`.
///
/// Returns the deserialized list and the number of word entries consumed.
pub fn deserialize_list(words: &[String], start: usize) -> (CompoundList, usize) {
    let mut list: CompoundList = Vec::new();
    let mut i = start;

    while i < words.len() {
        if words[i] != TAG_CMD { break; }
        i += 1; // skip TAG_CMD

        let negate = words.get(i).map(|s| s.as_str()) == Some(TAG_NEGATE);
        i += 1; // skip negate flag

        let cmd_count: usize = words.get(i)
            .and_then(|s| parse_usize(s.as_str()))
            .unwrap_or(0);
        i += 1;

        let mut commands: Vec<SimpleCommand> = Vec::new();
        for _ in 0..cmd_count {
            // assignments
            let nassign = words.get(i).and_then(|s| parse_usize(s)).unwrap_or(0);
            i += 1;
            let mut assignments: Vec<String> = Vec::new();
            for _ in 0..nassign { assignments.push(words.get(i).cloned().unwrap_or_default()); i += 1; }

            // words
            let nwords = words.get(i).and_then(|s| parse_usize(s)).unwrap_or(0);
            i += 1;
            let mut cmd_words: Vec<String> = Vec::new();
            for _ in 0..nwords { cmd_words.push(words.get(i).cloned().unwrap_or_default()); i += 1; }

            // redirects
            let nredirs = words.get(i).and_then(|s| parse_usize(s)).unwrap_or(0);
            i += 1;
            let mut redirects: Vec<Redirect> = Vec::new();
            for _ in 0..nredirs {
                let (r, consumed) = deserialize_redirect(words, i);
                redirects.push(r);
                i += consumed;
            }

            commands.push(SimpleCommand { assignments, words: cmd_words, redirects });
        }

        // separator tag
        let separator = match words.get(i).map(|s| s.as_str()) {
            Some(TAG_AND)  => Separator::And,
            Some(TAG_OR)   => Separator::Or,
            Some(TAG_AMP)  => Separator::Amp,
            Some(TAG_SEMI) => Separator::Semi,
            _              => Separator::End,
        };
        i += 1;

        list.push(AndOrItem {
            pipeline: Pipeline { negate, commands },
            separator,
        });
    }

    (list, i - start)
}

fn deserialize_redirect(words: &[String], i: usize) -> (Redirect, usize) {
    let tag = words.get(i).map(|s| s.as_str()).unwrap_or("");
    let fd  = parse_opt_fd(words.get(i + 1).map(|s| s.as_str()).unwrap_or("_"));

    macro_rules! two_arg {
        ($variant:ident) => {{
            let arg = words.get(i + 2).cloned().unwrap_or_default();
            (Redirect::$variant(fd, arg), 3)
        }};
    }

    match tag {
        "StdinFrom"       => two_arg!(StdinFrom),
        "StdoutTo"        => two_arg!(StdoutTo),
        "StdoutAppend"    => two_arg!(StdoutAppend),
        "StdoutNoclobber" => two_arg!(StdoutNoclobber),
        "DupOut"          => two_arg!(DupOut),
        "DupIn"           => two_arg!(DupIn),
        "ReadWrite"       => two_arg!(ReadWrite),
        "HereDoc" => {
            let strip = words.get(i + 2).map(|s| s.as_str()) == Some("1");
            let body  = words.get(i + 3).cloned().unwrap_or_default();
            (Redirect::HereDoc(fd, strip, body), 4)
        }
        _ => (Redirect::StdinFrom(None, String::new()), 1),
    }
}

fn parse_opt_fd(s: &str) -> Option<u32> {
    if s == "_" { None } else { parse_u32(s) }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for c in s.chars() {
        let d = c.to_digit(10)?;
        result = result.checked_mul(10)?.checked_add(d)?;
    }
    Some(result)
}

fn parse_usize(s: &str) -> Option<usize> {
    parse_u32(s).map(|n| n as usize)
}
