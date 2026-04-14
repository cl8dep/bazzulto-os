// sh-tests/tests/lexer.rs — unit tests for sh/lexer.rs
//
// The lexer has no crate-level dependencies outside alloc, so it compiles
// cleanly under std by re-using the same source file.

extern crate alloc;

// Shim: lexer.rs calls crate::write_err in read_heredoc_body.
pub fn write_err(_s: &str) {}

#[path = "../sh/src/lexer.rs"]
mod lexer;

use lexer::{tokenize, Token};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tok(input: &str) -> Vec<Token> {
    tokenize(input).expect("tokenize failed")
}

fn words(input: &str) -> Vec<String> {
    tok(input)
        .into_iter()
        .filter_map(|t| if let Token::Word(w) = t { Some(w) } else { None })
        .collect()
}

// ---------------------------------------------------------------------------
// §2.3 Basic tokenization
// ---------------------------------------------------------------------------

#[test]
fn empty_input_gives_no_tokens() {
    assert!(tok("").is_empty());
}

#[test]
fn single_word() {
    let t = tok("hello");
    assert_eq!(t, vec![Token::Word("hello".into())]);
}

#[test]
fn multiple_words_separated_by_spaces() {
    assert_eq!(words("foo bar baz"), vec!["foo", "bar", "baz"]);
}

#[test]
fn tabs_are_word_separators() {
    assert_eq!(words("a\tb"), vec!["a", "b"]);
}

#[test]
fn comment_discards_rest_of_line() {
    assert_eq!(words("foo # this is a comment"), vec!["foo"]);
}

#[test]
fn comment_at_start_discards_everything() {
    assert!(words("# full comment line").is_empty());
}

// ---------------------------------------------------------------------------
// §2.2 Quoting
// ---------------------------------------------------------------------------

#[test]
fn single_quoted_string() {
    // Single quotes: everything literal, including spaces and $
    assert_eq!(words("'hello world'"), vec!["hello world"]);
}

#[test]
fn single_quoted_preserves_dollar() {
    assert_eq!(words("'$HOME'"), vec!["$HOME"]);
}

#[test]
fn double_quoted_string() {
    assert_eq!(words("\"hello world\""), vec!["hello world"]);
}

#[test]
fn double_quoted_preserves_dollar_sign() {
    // $ inside double quotes keeps its literal representation for further expansion
    assert_eq!(words("\"$HOME\""), vec!["$HOME"]);
}

#[test]
fn backslash_escapes_space() {
    assert_eq!(words("hello\\ world"), vec!["hello world"]);
}

#[test]
fn backslash_newline_is_line_continuation() {
    // §2.2.1: backslash-newline = line continuation, both removed
    assert_eq!(words("hel\\\nlo"), vec!["hello"]);
}

#[test]
fn adjacent_quotes_concatenate() {
    assert_eq!(words("'foo''bar'"), vec!["foobar"]);
}

#[test]
fn mixed_quoting() {
    assert_eq!(words("'a'\"b\"c"), vec!["abc"]);
}

// ---------------------------------------------------------------------------
// §2.3 Operator tokens
// ---------------------------------------------------------------------------

#[test]
fn pipe_operator() {
    assert!(matches!(tok("a | b")[1], Token::Pipe));
}

#[test]
fn and_and_operator() {
    assert!(matches!(tok("a && b")[1], Token::AmpAmp));
}

#[test]
fn or_or_operator() {
    assert!(matches!(tok("a || b")[1], Token::PipeOr));
}

#[test]
fn semicolon_operator() {
    assert!(matches!(tok("a ; b")[1], Token::Semi));
}

#[test]
fn amp_operator() {
    assert!(matches!(tok("a &")[1], Token::Amp));
}

#[test]
fn double_semicolon_operator() {
    assert!(matches!(tok("a ;; b")[1], Token::SemiSemi));
}

#[test]
fn lparen_rparen() {
    let t = tok("(a)");
    assert!(matches!(t[0], Token::LParen));
    assert!(matches!(t[2], Token::RParen));
}

// ---------------------------------------------------------------------------
// §2.7 Redirect operators
// ---------------------------------------------------------------------------

#[test]
fn redirect_in() {
    let t = tok("< file");
    assert!(matches!(t[0], Token::RedirIn(None)));
}

#[test]
fn redirect_out() {
    let t = tok("> file");
    assert!(matches!(t[0], Token::RedirOut(None)));
}

#[test]
fn redirect_append() {
    let t = tok(">> file");
    assert!(matches!(t[0], Token::RedirAppend(None)));
}

#[test]
fn redirect_noclobber() {
    let t = tok(">| file");
    assert!(matches!(t[0], Token::RedirOutNoclobber(None)));
}

#[test]
fn redirect_dup_out() {
    let t = tok(">&2");
    assert!(matches!(t[0], Token::RedirDupOut(None)));
}

#[test]
fn redirect_dup_in() {
    let t = tok("<&0");
    assert!(matches!(t[0], Token::RedirDupIn(None)));
}

#[test]
fn redirect_read_write() {
    let t = tok("<> file");
    assert!(matches!(t[0], Token::RedirReadWrite(None)));
}

#[test]
fn heredoc_operator() {
    let t = tok("<< EOF");
    assert!(matches!(t[0], Token::HereDoc(None, false)));
    assert_eq!(t[1], Token::Word("EOF".into()));
}

#[test]
fn heredoc_strip_tabs() {
    let t = tok("<<- EOF");
    assert!(matches!(t[0], Token::HereDoc(None, true)));
}

#[test]
fn explicit_fd_on_redirect() {
    let t = tok("2> err");
    assert!(matches!(t[0], Token::RedirOut(Some(2))));
}

// ---------------------------------------------------------------------------
// §2.4 Reserved words
// ---------------------------------------------------------------------------

#[test]
fn reserved_words_are_plain_word_tokens() {
    // The lexer tokenizes reserved words as Token::Word — the parser elevates them.
    for rw in &["if", "then", "else", "elif", "fi", "for", "in", "do",
                "done", "case", "esac", "while", "until", "{", "}", "!"] {
        let t = tok(rw);
        assert!(
            matches!(&t[0], Token::Word(w) if w.as_str() == *rw),
            "reserved word '{}' should tokenize as Word",
            rw
        );
    }
}

// ---------------------------------------------------------------------------
// §2.4 is_reserved_word
// ---------------------------------------------------------------------------

#[test]
fn is_reserved_word_true_for_keywords() {
    use lexer::is_reserved_word;
    for w in &["if", "fi", "do", "done", "case", "esac", "while", "until",
               "for", "in", "then", "else", "elif", "!", "{", "}"] {
        assert!(is_reserved_word(w), "'{}' should be a reserved word", w);
    }
}

#[test]
fn is_reserved_word_false_for_regular_words() {
    use lexer::is_reserved_word;
    assert!(!is_reserved_word("echo"));
    assert!(!is_reserved_word("foo"));
    assert!(!is_reserved_word(""));
}
