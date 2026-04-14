//! Minimal TOML parser for bzinit service files.
//!
//! Parses only the subset used by `.service` files:
//!   - `[section]` headers
//!   - `key = "string value"`
//!   - `key = ["item1", "item2"]`
//!
//! Input is `&[u8]` from a ramfs read. Output is `ParsedToml` — a flat list
//! of (section, key, value) triples. Section names and keys are `&str`
//! slices into the input; string values are owned `String` (unescaped).

use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// A single parsed value — either a scalar string or an array of strings.
#[derive(Debug, Clone)]
pub enum TomlValue {
    String(String),
    Array(Vec<String>),
}

/// A flat list of (section, key, value) triples from a TOML file.
#[derive(Debug, Default)]
pub struct ParsedToml {
    pub entries: Vec<(String, String, TomlValue)>,
}

impl ParsedToml {
    /// Return the first matching string value for `section.key`, if any.
    pub fn get_string(&self, section: &str, key: &str) -> Option<&str> {
        for (s, k, v) in &self.entries {
            if s == section && k == key {
                if let TomlValue::String(ref string_value) = v {
                    return Some(string_value.as_str());
                }
            }
        }
        None
    }

    /// Return the first matching array for `section.key`, or an empty slice.
    pub fn get_array(&self, section: &str, key: &str) -> &[String] {
        for (s, k, v) in &self.entries {
            if s == section && k == key {
                if let TomlValue::Array(ref array_value) = v {
                    return array_value.as_slice();
                }
            }
        }
        &[]
    }
}

// ---------------------------------------------------------------------------
// Manual trim — avoids str::trim_matches and the CharSearcher codegen
// ---------------------------------------------------------------------------

fn trim_ascii(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_ws(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && is_ws(bytes[end - 1]) {
        end -= 1;
    }
    // SAFETY: we only advance past ASCII whitespace bytes, which are all
    // single-byte UTF-8 code points, so the resulting slice is still valid UTF-8.
    unsafe { core::str::from_utf8_unchecked(&bytes[start..end]) }
}

#[inline(always)]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse `input` as a TOML service file. Silently skips unrecognized lines.
pub fn parse(input: &[u8]) -> ParsedToml {

    let mut result = ParsedToml::default();
    let mut current_section = String::new();

    // Manual line iterator — avoids str::lines() / CharSearcher codegen that
    // corrupts callee-saved registers holding the vDSO write address.
    let mut pos = 0usize;
    while pos <= input.len() {
        // Find end of current line.
        let line_start = pos;
        let mut line_end = pos;
        while line_end < input.len() && input[line_end] != b'\n' {
            line_end += 1;
        }
        pos = line_end + 1; // advance past '\n' (or past end on last line)

        // Skip empty last segment.
        if line_start >= input.len() {
            break;
        }

        // Convert line bytes to &str; skip non-UTF-8 lines silently.
        let raw_bytes = &input[line_start..line_end];
        let raw_line = match core::str::from_utf8(raw_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let line = trim_ascii(raw_line);

        // Skip blank lines and comments.
        if line.is_empty() || line.as_bytes()[0] == b'#' {
            continue;
        }

        // Section header: [section]
        let line_bytes = line.as_bytes();
        if line_bytes[0] == b'[' && line_bytes[line_bytes.len() - 1] == b']' {
            current_section = String::from(&line[1..line.len() - 1]);
            continue;
        }

        // Key-value pair: key = ...
        // Find '=' manually.
        let mut eq_pos = line_bytes.len(); // sentinel: not found
        for (i, &b) in line_bytes.iter().enumerate() {
            if b == b'=' {
                eq_pos = i;
                break;
            }
        }
        if eq_pos == line_bytes.len() {
            continue; // no '=' found
        }

        let key = trim_ascii(&line[..eq_pos]);
        let value_str = trim_ascii(&line[eq_pos + 1..]);

        if key.is_empty() || value_str.is_empty() {
            continue;
        }


        let first = value_str.as_bytes()[0];
        let parsed_value = if first == b'[' {
            TomlValue::Array(parse_string_array(value_str))
        } else if first == b'"' {
            TomlValue::String(parse_quoted_string(value_str))
        } else {
            TomlValue::String(String::from(value_str))
        };

        result.entries.push((
            current_section.clone(),
            String::from(key),
            parsed_value,
        ));
    }

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `"string value"` → `String` (strips outer quotes, no escape handling).
fn parse_quoted_string(input: &str) -> String {
    let stripped = trim_ascii(input);
    let bytes = stripped.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        String::from(&stripped[1..stripped.len() - 1])
    } else {
        String::from(stripped)
    }
}

/// Parse `["a", "b"]` → `Vec<String>`.
fn parse_string_array(input: &str) -> Vec<String> {
    let trimmed = trim_ascii(input);
    // Strip surrounding '[' and ']'.
    let inner = if trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        return Vec::new();
    };

    let mut result = Vec::new();
    // Split on ',' without using str::split (which also uses CharSearcher).
    let bytes = inner.as_bytes();
    let mut start = 0usize;
    loop {
        let mut end = start;
        while end < bytes.len() && bytes[end] != b',' {
            end += 1;
        }
        let item = trim_ascii(match core::str::from_utf8(&bytes[start..end]) {
            Ok(s) => s,
            Err(_) => {
                if end >= bytes.len() { break; }
                start = end + 1;
                continue;
            }
        });
        let value = parse_quoted_string(item);
        if !value.is_empty() {
            result.push(value);
        }
        if end >= bytes.len() {
            break;
        }
        start = end + 1;
    }
    result
}
