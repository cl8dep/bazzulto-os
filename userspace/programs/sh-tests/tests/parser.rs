// sh-tests/tests/parser.rs — unit tests for sh/parser.rs
//
// parser.rs uses `crate::lexer` and `crate::vars`. Both are included here
// so that `crate::lexer` and `crate::vars` resolve correctly when parser.rs
// is compiled as part of this test binary.

extern crate alloc;

// Shim: lexer.rs calls crate::write_err in read_heredoc_body.
pub fn write_err(_s: &str) {}

// Minimal ShellState for vars::expand_special (used transitively by vars.rs).
pub struct ShellState {
    pub last_exit_status:    i32,
    pub shell_name:          String,
    pub shell_pid:           u32,
    pub last_background_pid: Option<i32>,
    pub positional_params:   Vec<String>,
}

#[path = "../sh/src/lexer.rs"]
mod lexer;
#[path = "../sh/src/vars.rs"]
mod vars;
#[path = "../sh/src/parser.rs"]
mod parser;

use lexer::tokenize;
use parser::{
    parse_compound_list, CompoundList, AndOrItem, Pipeline, SimpleCommand,
    Redirect, Separator, ParseError,
    TAG_SUBSHELL, TAG_GROUP, TAG_FOR,
    TAG_IF, TAG_THEN, TAG_ELIF, TAG_ELSE, TAG_FI,
    TAG_WHILE, TAG_UNTIL, TAG_BODY,
    TAG_CASE, TAG_CASE_ITEM, TAG_ESAC,
    TAG_FUNCDEF,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse(input: &str) -> CompoundList {
    let tokens = tokenize(input).expect("tokenize failed");
    parse_compound_list(&tokens).expect("parse failed")
}

fn parse_err(input: &str) -> ParseError {
    let tokens = tokenize(input).expect("tokenize failed");
    parse_compound_list(&tokens).unwrap_err()
}

fn single_pipeline(list: &CompoundList) -> &Pipeline {
    assert_eq!(list.len(), 1, "expected exactly one list item");
    &list[0].pipeline
}

fn single_command(list: &CompoundList) -> &SimpleCommand {
    let pipeline = single_pipeline(list);
    assert_eq!(pipeline.commands.len(), 1, "expected single-command pipeline");
    &pipeline.commands[0]
}

// ---------------------------------------------------------------------------
// §2.9.1 Simple commands
// ---------------------------------------------------------------------------

#[test]
fn simple_command_words() {
    let list = parse("echo hello world");
    let cmd = single_command(&list);
    assert_eq!(cmd.words, vec!["echo", "hello", "world"]);
    assert!(cmd.assignments.is_empty());
    assert!(cmd.redirects.is_empty());
}

#[test]
fn assignment_only() {
    let list = parse("FOO=bar");
    let cmd = single_command(&list);
    assert_eq!(cmd.assignments, vec!["FOO=bar"]);
    assert!(cmd.words.is_empty());
}

#[test]
fn assignment_before_command() {
    let list = parse("FOO=bar echo hello");
    let cmd = single_command(&list);
    assert_eq!(cmd.assignments, vec!["FOO=bar"]);
    assert_eq!(cmd.words, vec!["echo", "hello"]);
}

#[test]
fn redirect_stdin() {
    let list = parse("cat < file.txt");
    let cmd = single_command(&list);
    assert_eq!(cmd.words, vec!["cat"]);
    assert!(matches!(cmd.redirects[0], Redirect::StdinFrom(None, ref f) if f == "file.txt"));
}

#[test]
fn redirect_stdout() {
    let list = parse("echo hi > out.txt");
    let cmd = single_command(&list);
    assert!(matches!(cmd.redirects[0], Redirect::StdoutTo(None, ref f) if f == "out.txt"));
}

#[test]
fn redirect_append() {
    let list = parse("echo hi >> out.txt");
    let cmd = single_command(&list);
    assert!(matches!(cmd.redirects[0], Redirect::StdoutAppend(None, ref f) if f == "out.txt"));
}

#[test]
fn explicit_fd_redirect() {
    let list = parse("cmd 2> err.txt");
    let cmd = single_command(&list);
    assert!(matches!(cmd.redirects[0], Redirect::StdoutTo(Some(2), _)));
}

// ---------------------------------------------------------------------------
// §2.9.2 Pipelines
// ---------------------------------------------------------------------------

#[test]
fn two_stage_pipeline() {
    let tokens = tokenize("cat file | wc -l").unwrap();
    let list = parse_compound_list(&tokens).unwrap();
    let pipeline = single_pipeline(&list);
    assert_eq!(pipeline.commands.len(), 2);
    assert_eq!(pipeline.commands[0].words[0], "cat");
    assert_eq!(pipeline.commands[1].words[0], "wc");
}

#[test]
fn pipeline_negate() {
    let tokens = tokenize("! false").unwrap();
    let list = parse_compound_list(&tokens).unwrap();
    let pipeline = single_pipeline(&list);
    assert!(pipeline.negate);
}

#[test]
fn pipeline_no_negate_by_default() {
    let list = parse("true");
    assert!(!single_pipeline(&list).negate);
}

// ---------------------------------------------------------------------------
// §2.9.3 Lists — &&, ||, ;, &
// ---------------------------------------------------------------------------

#[test]
fn and_and_list() {
    let tokens = tokenize("a && b").unwrap();
    let list = parse_compound_list(&tokens).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].separator, Separator::And);
    assert_eq!(list[0].pipeline.commands[0].words[0], "a");
    assert_eq!(list[1].pipeline.commands[0].words[0], "b");
}

#[test]
fn or_or_list() {
    let tokens = tokenize("a || b").unwrap();
    let list = parse_compound_list(&tokens).unwrap();
    assert_eq!(list[0].separator, Separator::Or);
}

#[test]
fn semicolon_terminates_item() {
    // In TopLevel context, `;` ends the statement; next statement in new parse.
    // The parser (TopLevel) breaks after Semi, so we get one item with Semi separator.
    let tokens = tokenize("a ; b").unwrap();
    // We can parse both statements by accumulating tokens:
    // In the REPL, each statement is parsed individually. Here we test that
    // the semicolon is parsed correctly as part of the first item.
    let list = parse_compound_list(&tokens).unwrap();
    // TopLevel context: we stop after the first statement.
    assert!(!list.is_empty());
    assert_eq!(list[0].pipeline.commands[0].words[0], "a");
}

#[test]
fn amp_separator_marks_background() {
    let tokens = tokenize("sleep 1 &").unwrap();
    let list = parse_compound_list(&tokens).unwrap();
    assert_eq!(list[0].separator, Separator::Amp);
}

// ---------------------------------------------------------------------------
// §2.9.4.1 Subshell ( )
// ---------------------------------------------------------------------------

#[test]
fn subshell_tag() {
    let list = parse("(echo hi)");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_SUBSHELL);
}

#[test]
fn unclosed_subshell_needs_more() {
    let err = parse_err("(echo hi");
    assert_eq!(err, ParseError::NeedMore);
}

// ---------------------------------------------------------------------------
// §2.9.4.1 Group { }
// ---------------------------------------------------------------------------

#[test]
fn group_tag() {
    let list = parse("{ echo hi; }");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_GROUP);
}

#[test]
fn unclosed_group_needs_more() {
    let err = parse_err("{ echo hi");
    assert_eq!(err, ParseError::NeedMore);
}

// ---------------------------------------------------------------------------
// §2.9.4.2 For loop
// ---------------------------------------------------------------------------

#[test]
fn for_loop_tag() {
    let list = parse("for x in a b c; do echo $x; done");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_FOR);
}

#[test]
fn for_loop_variable_name() {
    let list = parse("for myvar in 1 2; do :; done");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[1], "myvar");
}

#[test]
fn for_loop_word_list() {
    let list = parse("for x in a b c; do :; done");
    let cmd = single_command(&list);
    // words[2] = "some:3", words[3..5] = "a" "b" "c"
    assert!(cmd.words[2].starts_with("some:3"));
    assert_eq!(cmd.words[3], "a");
    assert_eq!(cmd.words[4], "b");
    assert_eq!(cmd.words[5], "c");
}

#[test]
fn for_loop_no_in_clause() {
    let list = parse("for x; do :; done");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[2], "none");
}

#[test]
fn unclosed_for_loop_needs_more() {
    let err = parse_err("for x in a b; do echo $x");
    assert_eq!(err, ParseError::NeedMore);
}

// ---------------------------------------------------------------------------
// ParseError::NeedMore propagation
// ---------------------------------------------------------------------------

#[test]
fn need_more_for_unclosed_subshell() {
    let tokens = tokenize("(").unwrap();
    let err = parse_compound_list(&tokens).unwrap_err();
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn need_more_for_unclosed_group() {
    let tokens = tokenize("{").unwrap();
    let err = parse_compound_list(&tokens).unwrap_err();
    assert_eq!(err, ParseError::NeedMore);
}

// ---------------------------------------------------------------------------
// Multi-token accumulation simulating REPL continuation
// ---------------------------------------------------------------------------

#[test]
fn multi_line_for_loop() {
    // Simulate two lines being accumulated into one token buffer.
    use lexer::Token;
    let line1 = tokenize("for x in a b").unwrap();
    let line2 = tokenize("do echo $x; done").unwrap();

    let mut all_tokens = line1;
    all_tokens.push(Token::Newline);
    all_tokens.extend(line2);
    all_tokens.push(Token::Newline);

    let list = parse_compound_list(&all_tokens).expect("multi-line for loop parse failed");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_FOR);
}

// ---------------------------------------------------------------------------
// §2.9.4.4 if / elif / else / fi
// ---------------------------------------------------------------------------

#[test]
fn if_tag() {
    let list = parse("if true; then echo yes; fi");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_IF);
}

#[test]
fn if_contains_then_tag() {
    let list = parse("if true; then echo yes; fi");
    let cmd = single_command(&list);
    // Serialized form: TAG_IF <cond_list> TAG_THEN <then_list> TAG_FI
    assert!(cmd.words.contains(&TAG_THEN.to_string()),
        "expected TAG_THEN in if words, got: {:?}", cmd.words);
}

#[test]
fn if_contains_fi_tag() {
    let list = parse("if true; then echo yes; fi");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_FI.to_string()),
        "expected TAG_FI in if words, got: {:?}", cmd.words);
}

#[test]
fn if_else() {
    let list = parse("if false; then echo no; else echo yes; fi");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_ELSE.to_string()),
        "expected TAG_ELSE in if/else words, got: {:?}", cmd.words);
    assert!(cmd.words.contains(&TAG_FI.to_string()));
}

#[test]
fn if_elif() {
    let list = parse("if false; then echo a; elif true; then echo b; fi");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_ELIF.to_string()),
        "expected TAG_ELIF in words, got: {:?}", cmd.words);
}

#[test]
fn if_elif_else() {
    let list = parse("if false; then echo a; elif false; then echo b; else echo c; fi");
    let cmd = single_command(&list);
    let words = &cmd.words;
    assert!(words.contains(&TAG_ELIF.to_string()));
    assert!(words.contains(&TAG_ELSE.to_string()));
    assert!(words.contains(&TAG_FI.to_string()));
}

#[test]
fn unclosed_if_needs_more() {
    let err = parse_err("if true; then echo yes");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn if_without_then_needs_more() {
    let err = parse_err("if true");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn multi_line_if() {
    use lexer::Token;
    let line1 = tokenize("if true").unwrap();
    let line2 = tokenize("then echo yes").unwrap();
    let line3 = tokenize("fi").unwrap();
    let mut all = line1;
    all.push(Token::Newline);
    all.extend(line2);
    all.push(Token::Newline);
    all.extend(line3);
    all.push(Token::Newline);
    let list = parse_compound_list(&all).expect("multi-line if parse failed");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_IF);
}

// ---------------------------------------------------------------------------
// §2.9.4.5/.6 while / until
// ---------------------------------------------------------------------------

#[test]
fn while_tag() {
    let list = parse("while true; do echo loop; done");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_WHILE);
}

#[test]
fn until_tag() {
    let list = parse("until false; do echo loop; done");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_UNTIL);
}

#[test]
fn while_contains_body_tag() {
    let list = parse("while true; do echo loop; done");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_BODY.to_string()),
        "expected TAG_BODY in while words, got: {:?}", cmd.words);
}

#[test]
fn unclosed_while_needs_more() {
    let err = parse_err("while true; do echo loop");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn unclosed_until_needs_more() {
    let err = parse_err("until false; do echo loop");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn multi_line_while() {
    use lexer::Token;
    let line1 = tokenize("while true").unwrap();
    let line2 = tokenize("do echo loop").unwrap();
    let line3 = tokenize("done").unwrap();
    let mut all = line1;
    all.push(Token::Newline);
    all.extend(line2);
    all.push(Token::Newline);
    all.extend(line3);
    all.push(Token::Newline);
    let list = parse_compound_list(&all).expect("multi-line while parse failed");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_WHILE);
}

// ---------------------------------------------------------------------------
// §2.9.4.3 case...esac
// ---------------------------------------------------------------------------

#[test]
fn case_tag() {
    let list = parse("case foo in bar) echo no;; foo) echo yes;; esac");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_CASE);
}

#[test]
fn case_subject_word() {
    let list = parse("case myvar in x) :;; esac");
    let cmd = single_command(&list);
    // words[1] is the subject word
    assert_eq!(cmd.words[1], "myvar");
}

#[test]
fn case_contains_esac_tag() {
    let list = parse("case x in a) echo a;; esac");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_ESAC.to_string()),
        "expected TAG_ESAC in case words, got: {:?}", cmd.words);
}

#[test]
fn case_contains_case_item_tag() {
    let list = parse("case x in a) echo a;; esac");
    let cmd = single_command(&list);
    assert!(cmd.words.contains(&TAG_CASE_ITEM.to_string()),
        "expected TAG_CASE_ITEM in case words, got: {:?}", cmd.words);
}

#[test]
fn case_multiple_patterns() {
    let list = parse("case x in a|b) echo ab;; esac");
    let cmd = single_command(&list);
    // TAG_CASE_ITEM then N=2 then "a" "b"
    let case_item_pos = cmd.words.iter().position(|w| w == TAG_CASE_ITEM)
        .expect("TAG_CASE_ITEM not found");
    let n: usize = cmd.words[case_item_pos + 1].parse().expect("pattern count should be integer");
    assert_eq!(n, 2, "expected 2 patterns for 'a|b'");
    assert_eq!(cmd.words[case_item_pos + 2], "a");
    assert_eq!(cmd.words[case_item_pos + 3], "b");
}

#[test]
fn case_empty_body_esac() {
    // A case with no patterns (empty body) must parse to TAG_CASE + subject + TAG_ESAC
    let list = parse("case x in esac");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_CASE);
    assert_eq!(cmd.words[1], "x");
    assert!(cmd.words.contains(&TAG_ESAC.to_string()));
}

#[test]
fn unclosed_case_needs_more() {
    let err = parse_err("case x in a) echo a");
    assert_eq!(err, ParseError::NeedMore);
}

// ---------------------------------------------------------------------------
// §2.9.5 Function definitions
// ---------------------------------------------------------------------------

#[test]
fn funcdef_tag() {
    let list = parse("f() { echo hello; }");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_FUNCDEF);
}

#[test]
fn funcdef_name() {
    let list = parse("my_func() { echo hello; }");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[1], "my_func");
}

#[test]
fn funcdef_body_is_group() {
    // The body (TAG_GROUP + serialized content) starts at words[2].
    let list = parse("f() { echo hello; }");
    let cmd = single_command(&list);
    assert!(cmd.words.len() >= 3, "expected body words after name");
    assert_eq!(cmd.words[2], TAG_GROUP);
}

#[test]
fn funcdef_body_is_subshell() {
    // A subshell ( ) is also a valid function body per POSIX.
    let list = parse("f() (echo hello)");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_FUNCDEF);
    assert_eq!(cmd.words[1], "f");
    assert_eq!(cmd.words[2], TAG_SUBSHELL);
}

#[test]
fn funcdef_missing_body_needs_more() {
    let err = parse_err("f()");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn funcdef_unclosed_body_needs_more() {
    let err = parse_err("f() { echo hello");
    assert_eq!(err, ParseError::NeedMore);
}

#[test]
fn funcdef_multiline() {
    use lexer::Token;
    let line1 = tokenize("greet()").unwrap();
    let line2 = tokenize("{ echo hi; }").unwrap();
    let mut all = line1;
    all.push(Token::Newline);
    all.extend(line2);
    all.push(Token::Newline);
    let list = parse_compound_list(&all).expect("multi-line funcdef failed");
    let cmd = single_command(&list);
    assert_eq!(cmd.words[0], TAG_FUNCDEF);
    assert_eq!(cmd.words[1], "greet");
}
