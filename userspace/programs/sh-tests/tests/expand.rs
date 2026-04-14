// sh-tests/tests/expand.rs — unit tests for sh/expand.rs
//
// expand.rs uses `crate::ShellState`, `crate::vars`, and `crate::write_err`.
// All are provided here before including the source files.

extern crate alloc;

// Shim: expand.rs calls crate::exit_on_error in the ${var:?} error path.
// In tests, this panics so the test harness can catch it with should_panic.
pub fn exit_on_error(_code: i32) -> ! {
    panic!("exit_on_error called in test context")
}

// Minimal ShellState matching the fields that expand.rs and vars.rs use.
pub struct ShellState {
    pub last_exit_status:    i32,
    pub shell_name:          String,
    pub shell_pid:           u32,
    pub last_background_pid: Option<i32>,
    pub positional_params:   Vec<String>,
    pub vars:                vars::VarStore,
    pub is_interactive:      bool,
    pub pipefail:            bool,
    pub option_nounset:      bool,
    pub command_sub_fn:      fn(&str, &mut ShellState) -> String,
}

fn noop_command_sub(_cmd: &str, _state: &mut ShellState) -> String {
    String::new()
}

impl ShellState {
    fn new() -> Self {
        let mut s = ShellState {
            last_exit_status:    0,
            shell_name:          "sh".into(),
            shell_pid:           1,
            last_background_pid: None,
            positional_params:   Vec::new(),
            vars:                vars::VarStore::new(),
            is_interactive:      false,
            pipefail:            false,
            option_nounset:      false,
            command_sub_fn:      noop_command_sub,
        };
        s.vars.set("HOME", "/home/user").unwrap();
        s.vars.set("IFS", " \t\n").unwrap();
        s
    }
}

// write_err shim: discard in tests (${var:?} error output goes nowhere)
pub fn write_err(_s: &str) {}

#[path = "../sh/src/vars.rs"]
mod vars;
#[path = "../sh/src/expand.rs"]
mod expand;

use expand::{expand_word, expand_word_nosplit};

// ---------------------------------------------------------------------------
// §2.6.1 Tilde expansion
// ---------------------------------------------------------------------------

#[test]
fn tilde_alone_expands_to_home() {
    let mut state = ShellState::new();
    let fields = expand_word("~", &mut state);
    assert_eq!(fields, vec!["/home/user"]);
}

#[test]
fn tilde_slash_path() {
    let mut state = ShellState::new();
    let fields = expand_word("~/docs", &mut state);
    assert_eq!(fields, vec!["/home/user/docs"]);
}

#[test]
fn no_tilde_unchanged() {
    let mut state = ShellState::new();
    let fields = expand_word("hello", &mut state);
    assert_eq!(fields, vec!["hello"]);
}

// ---------------------------------------------------------------------------
// §2.6.2 Parameter expansion — basic $VAR and ${VAR}
// ---------------------------------------------------------------------------

#[test]
fn expand_set_variable() {
    let mut state = ShellState::new();
    state.vars.set("GREETING", "hello").unwrap();
    assert_eq!(expand_word("$GREETING", &mut state), vec!["hello"]);
}

#[test]
fn expand_set_variable_braces() {
    let mut state = ShellState::new();
    state.vars.set("X", "42").unwrap();
    assert_eq!(expand_word("${X}", &mut state), vec!["42"]);
}

#[test]
fn expand_unset_variable_is_empty() {
    let mut state = ShellState::new();
    // IFS is " \t\n" — empty expansion produces no fields
    let fields = expand_word("$UNSET", &mut state);
    // An unset variable expands to empty string; field splitting produces nothing.
    assert!(fields.is_empty() || fields == vec!["".to_string()]);
}

// ---------------------------------------------------------------------------
// §2.6.2 Parameter expansion operators
// ---------------------------------------------------------------------------

#[test]
fn default_value_when_unset() {
    let mut state = ShellState::new();
    assert_eq!(expand_word_nosplit("${UNSET:-default}", &mut state), "default");
}

#[test]
fn default_value_not_used_when_set() {
    let mut state = ShellState::new();
    state.vars.set("VAR", "value").unwrap();
    assert_eq!(expand_word_nosplit("${VAR:-default}", &mut state), "value");
}

#[test]
fn assign_default_when_unset() {
    let mut state = ShellState::new();
    let result = expand_word_nosplit("${NEWVAR:=assigned}", &mut state);
    assert_eq!(result, "assigned");
    // Variable should now be set
    assert_eq!(state.vars.get("NEWVAR"), Some("assigned"));
}

#[test]
fn alternate_value_when_set() {
    let mut state = ShellState::new();
    state.vars.set("FLAG", "1").unwrap();
    assert_eq!(expand_word_nosplit("${FLAG:+yes}", &mut state), "yes");
}

#[test]
fn alternate_value_empty_when_unset() {
    let mut state = ShellState::new();
    assert_eq!(expand_word_nosplit("${UNSET:+yes}", &mut state), "");
}

#[test]
fn string_length_operator() {
    let mut state = ShellState::new();
    state.vars.set("STR", "hello").unwrap();
    assert_eq!(expand_word_nosplit("${#STR}", &mut state), "5");
}

#[test]
fn string_length_of_unset_is_zero() {
    let mut state = ShellState::new();
    assert_eq!(expand_word_nosplit("${#UNSET}", &mut state), "0");
}

// ---------------------------------------------------------------------------
// §2.5.2 Special parameters
// ---------------------------------------------------------------------------

#[test]
fn dollar_question_is_last_exit_status() {
    let mut state = ShellState::new();
    state.last_exit_status = 42;
    assert_eq!(expand_word_nosplit("$?", &mut state), "42");
}

#[test]
fn dollar_dollar_is_shell_pid() {
    let mut state = ShellState::new();
    state.shell_pid = 1234;
    assert_eq!(expand_word_nosplit("$$", &mut state), "1234");
}

#[test]
fn dollar_zero_is_shell_name() {
    let mut state = ShellState::new();
    state.shell_name = "mysh".into();
    assert_eq!(expand_word_nosplit("$0", &mut state), "mysh");
}

#[test]
fn dollar_hash_is_positional_param_count() {
    let mut state = ShellState::new();
    state.positional_params = vec!["a".into(), "b".into(), "c".into()];
    assert_eq!(expand_word_nosplit("$#", &mut state), "3");
}

#[test]
fn dollar_star_joins_positional_params() {
    let mut state = ShellState::new();
    state.positional_params = vec!["x".into(), "y".into()];
    assert_eq!(expand_word_nosplit("$*", &mut state), "x y");
}

// ---------------------------------------------------------------------------
// §2.5.1 Positional parameters $1, $2, ...
// ---------------------------------------------------------------------------

#[test]
fn positional_param_one() {
    let mut state = ShellState::new();
    state.positional_params = vec!["first".into(), "second".into()];
    assert_eq!(expand_word_nosplit("$1", &mut state), "first");
}

#[test]
fn positional_param_two() {
    let mut state = ShellState::new();
    state.positional_params = vec!["first".into(), "second".into()];
    assert_eq!(expand_word_nosplit("$2", &mut state), "second");
}

#[test]
fn positional_param_beyond_count_is_empty() {
    let mut state = ShellState::new();
    state.positional_params = vec!["only".into()];
    assert_eq!(expand_word_nosplit("$5", &mut state), "");
}

// ---------------------------------------------------------------------------
// §2.6.5 Field splitting
// ---------------------------------------------------------------------------

#[test]
fn field_split_on_space() {
    let mut state = ShellState::new();
    state.vars.set("MULTI", "a b c").unwrap();
    let fields = expand_word("$MULTI", &mut state);
    assert_eq!(fields, vec!["a", "b", "c"]);
}

#[test]
fn field_split_on_custom_ifs() {
    let mut state = ShellState::new();
    state.vars.set("IFS", ":").unwrap();
    state.vars.set("PATH_VAR", "a:b:c").unwrap();
    let fields = expand_word("$PATH_VAR", &mut state);
    assert_eq!(fields, vec!["a", "b", "c"]);
}

#[test]
fn no_field_split_in_double_quotes() {
    let mut state = ShellState::new();
    state.vars.set("MULTI", "a b c").unwrap();
    // The lexer strips the outer double quotes and marks the content as quoted.
    // After lexing "\"$MULTI\"" → word token `$MULTI` tagged as double-quoted.
    // For nosplit expand, there is no splitting regardless.
    assert_eq!(expand_word_nosplit("$MULTI", &mut state), "a b c");
}

// ---------------------------------------------------------------------------
// Word concatenation
// ---------------------------------------------------------------------------

#[test]
fn literal_concatenation() {
    let mut state = ShellState::new();
    assert_eq!(expand_word_nosplit("hello", &mut state), "hello");
}

#[test]
fn variable_in_the_middle_of_word() {
    let mut state = ShellState::new();
    state.vars.set("EXT", "txt").unwrap();
    assert_eq!(expand_word_nosplit("file.$EXT", &mut state), "file.txt");
}
