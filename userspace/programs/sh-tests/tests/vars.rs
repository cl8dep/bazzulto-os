// sh-tests/tests/vars.rs — unit tests for sh/vars.rs
//
// vars.rs references `crate::ShellState` only in `expand_special`. We provide
// a minimal ShellState here that satisfies the type requirements.

extern crate alloc;

// Shim: vars.rs doesn't call write_err directly, but lexer.rs (included by
// parser tests) does. Added here for completeness if vars is included standalone.
pub fn write_err(_s: &str) {}

// Minimal ShellState that satisfies vars::expand_special's requirements.
pub struct ShellState {
    pub last_exit_status:   i32,
    pub shell_name:         String,
    pub shell_pid:          u32,
    pub last_background_pid: Option<i32>,
    pub positional_params:  Vec<String>,
}

#[path = "../sh/src/vars.rs"]
mod vars;

use vars::{VarStore, is_valid_name, parse_assignment, format_u32};

// ---------------------------------------------------------------------------
// VarStore — get / set / unset
// ---------------------------------------------------------------------------

#[test]
fn set_and_get_variable() {
    let mut store = VarStore::new();
    store.set("FOO", "bar").unwrap();
    assert_eq!(store.get("FOO"), Some("bar"));
}

#[test]
fn get_unset_variable_returns_none() {
    let store = VarStore::new();
    assert_eq!(store.get("UNSET"), None);
}

#[test]
fn set_overwrites_existing_value() {
    let mut store = VarStore::new();
    store.set("X", "1").unwrap();
    store.set("X", "2").unwrap();
    assert_eq!(store.get("X"), Some("2"));
}

#[test]
fn unset_removes_variable() {
    let mut store = VarStore::new();
    store.set("Y", "hello").unwrap();
    store.unset("Y").unwrap();
    assert_eq!(store.get("Y"), None);
}

#[test]
fn unset_of_nonexistent_is_ok() {
    let mut store = VarStore::new();
    assert!(store.unset("MISSING").is_ok());
}

#[test]
fn is_set_true_when_set() {
    let mut store = VarStore::new();
    store.set("Z", "").unwrap();
    assert!(store.is_set("Z"));
}

#[test]
fn is_set_false_when_not_set() {
    let store = VarStore::new();
    assert!(!store.is_set("Z"));
}

// ---------------------------------------------------------------------------
// Readonly enforcement
// ---------------------------------------------------------------------------

#[test]
fn readonly_variable_cannot_be_overwritten() {
    let mut store = VarStore::new();
    store.set("RO", "value").unwrap();
    store.set_readonly("RO");
    assert!(store.set("RO", "new").is_err());
}

#[test]
fn readonly_variable_cannot_be_unset() {
    let mut store = VarStore::new();
    store.set("RO2", "value").unwrap();
    store.set_readonly("RO2");
    assert!(store.unset("RO2").is_err());
}

#[test]
fn non_readonly_variable_can_be_overwritten() {
    let mut store = VarStore::new();
    store.set("NRO", "old").unwrap();
    store.set("NRO", "new").unwrap();
    assert_eq!(store.get("NRO"), Some("new"));
}

// ---------------------------------------------------------------------------
// Export flag
// ---------------------------------------------------------------------------

#[test]
fn exported_variable_appears_in_for_each_exported() {
    let mut store = VarStore::new();
    store.set("EXPORTED", "yes").unwrap();
    store.export("EXPORTED");

    let mut found = false;
    store.for_each_exported(|name, value| {
        if name == "EXPORTED" && value == "yes" {
            found = true;
        }
    });
    assert!(found, "exported variable not found in for_each_exported");
}

#[test]
fn unexported_variable_not_in_for_each_exported() {
    let mut store = VarStore::new();
    store.set("HIDDEN", "secret").unwrap();

    let mut found = false;
    store.for_each_exported(|name, _| {
        if name == "HIDDEN" {
            found = true;
        }
    });
    assert!(!found, "unexported variable should not appear in for_each_exported");
}

#[test]
fn set_exported_marks_for_export() {
    let mut store = VarStore::new();
    store.set_exported("E", "1");

    let mut seen = false;
    store.for_each_exported(|name, _| {
        if name == "E" { seen = true; }
    });
    assert!(seen);
}

// ---------------------------------------------------------------------------
// is_valid_name
// ---------------------------------------------------------------------------

#[test]
fn valid_names() {
    for name in &["a", "A", "_", "abc", "ABC", "a1", "_1", "A_B_C", "x123"] {
        assert!(is_valid_name(name), "'{}' should be a valid name", name);
    }
}

#[test]
fn invalid_names() {
    for name in &["", "1", "1abc", "-foo", "a-b", "a.b", "a b"] {
        assert!(!is_valid_name(name), "'{}' should be invalid", name);
    }
}

// ---------------------------------------------------------------------------
// parse_assignment
// ---------------------------------------------------------------------------

#[test]
fn parse_valid_assignment() {
    assert_eq!(parse_assignment("FOO=bar"), Some(("FOO", "bar")));
    assert_eq!(parse_assignment("x="), Some(("x", "")));
    assert_eq!(parse_assignment("PATH=/usr/bin:/bin"), Some(("PATH", "/usr/bin:/bin")));
}

#[test]
fn parse_invalid_assignment_no_eq() {
    assert_eq!(parse_assignment("foo"), None);
}

#[test]
fn parse_invalid_assignment_bad_name() {
    assert_eq!(parse_assignment("1foo=bar"), None);
    assert_eq!(parse_assignment("-foo=bar"), None);
}

// ---------------------------------------------------------------------------
// format_u32
// ---------------------------------------------------------------------------

#[test]
fn format_u32_zero() {
    assert_eq!(format_u32(0), "0");
}

#[test]
fn format_u32_values() {
    assert_eq!(format_u32(1), "1");
    assert_eq!(format_u32(42), "42");
    assert_eq!(format_u32(1000000), "1000000");
    assert_eq!(format_u32(u32::MAX), "4294967295");
}
