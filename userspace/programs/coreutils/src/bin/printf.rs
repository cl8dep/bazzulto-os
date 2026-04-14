// POSIX.1-2024 — printf
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/printf.html
//
// Write formatted output to standard output.
//
// SYNOPSIS:  printf format [argument...]
//
// Supported conversion specifiers:
//   %d / %i   Signed decimal integer.
//   %u        Unsigned decimal integer.
//   %o        Unsigned octal integer.
//   %x / %X  Unsigned hexadecimal integer (lower / upper case).
//   %s        String.
//   %b        String with backslash-escape expansion (POSIX extension).
//             Supports \\ \a \b \f \n \r \t \v \0ddd \c (stop printing).
//   %c        First byte of argument string.
//   %%        Literal '%'.
//
// Flags:          - (left-align)  0 (zero-pad)  + (force sign)  <space>
// Width:          decimal integer
// Precision:      .decimal integer (min digits for integers; max bytes for s/b)
// Numbered args:  %n$ (e.g. %2$s)
//
// The format operand is reused as often as necessary to consume all arguments
// (POSIX §printf EXTENDED DESCRIPTION: "The format operand shall be reused as
// often as necessary to satisfy the argument operands.").
//
// Floating-point conversions (a A e E f F g G) are not required by POSIX for
// the printf *utility* and are not implemented.
//
// Exit codes:
//   0   Successful completion.
//  >0   An error occurred (bad conversion, overflow, etc.).

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::raw;
use coreutils::{args, write_stdout, write_stderr};

// ---------------------------------------------------------------------------
// Backslash-escape processing (format string escapes and %b argument escapes)
// ---------------------------------------------------------------------------

/// Process a single backslash escape sequence from `bytes` starting at `pos`
/// (which points past the '\'). Returns (output_byte_or_sentinel, chars_consumed).
///
/// Sentinel value 0x100 = '\c' (stop-printing marker — caller checks).
/// Sentinel value 0x101 = unknown escape (output '\' + the char literally).
fn process_escape(bytes: &[u8], pos: usize, allow_c: bool) -> (u32, usize) {
    if pos >= bytes.len() {
        return (b'\\' as u32, 0);
    }
    match bytes[pos] {
        b'\\' => (b'\\' as u32, 1),
        b'a'  => (0x07, 1),
        b'b'  => (0x08, 1),
        b'f'  => (0x0C, 1),
        b'n'  => (b'\n' as u32, 1),
        b'r'  => (b'\r' as u32, 1),
        b't'  => (b'\t' as u32, 1),
        b'v'  => (0x0B, 1),
        b'c'  => {
            if allow_c { (0x100, 1) } // '\c' stop sentinel
            else       { (0x101, 1) } // unknown in format string context
        }
        b'0' if allow_c => {
            // \0ddd — zero followed by up to three octal digits.
            let mut value = 0u32;
            let mut consumed = 1usize; // past the '0'
            for i in 0..3 {
                let idx = pos + 1 + i;
                if idx >= bytes.len() { break; }
                let byte = bytes[idx];
                if byte >= b'0' && byte <= b'7' {
                    value = value * 8 + (byte - b'0') as u32;
                    consumed += 1;
                } else {
                    break;
                }
            }
            (value & 0xFF, consumed)
        }
        digit if digit >= b'0' && digit <= b'7' => {
            // \ddd — one to three octal digits (format string context).
            let mut value = (digit - b'0') as u32;
            let mut consumed = 1usize;
            for i in 1..3 {
                let idx = pos + i;
                if idx >= bytes.len() { break; }
                let b = bytes[idx];
                if b >= b'0' && b <= b'7' {
                    value = value * 8 + (b - b'0') as u32;
                    consumed += 1;
                } else {
                    break;
                }
            }
            (value & 0xFF, consumed)
        }
        _ => (0x101, 0), // unknown — output '\' + char as-is
    }
}

/// Expand backslash escapes in a format string into `out`.
/// Returns false if a '\c' sentinel was encountered (stop printing).
fn expand_format_escapes(s: &str, out: &mut String) -> bool {
    let bytes = s.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] != b'\\' {
            out.push(bytes[pos] as char);
            pos += 1;
            continue;
        }
        pos += 1; // past '\'
        let (value, consumed) = process_escape(bytes, pos, false);
        pos += consumed;
        if value == 0x101 {
            // Unknown escape: output '\' then the char literally.
            out.push('\\');
            if consumed == 0 && pos < bytes.len() {
                out.push(bytes[pos] as char);
                pos += 1;
            }
        } else {
            out.push(value as u8 as char);
        }
    }
    true
}

/// Expand %b argument: like format escapes but '\c' is supported and causes
/// printing to stop. Returns false if '\c' was encountered.
fn expand_b_escapes(s: &str, out: &mut String) -> bool {
    let bytes = s.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] != b'\\' {
            out.push(bytes[pos] as char);
            pos += 1;
            continue;
        }
        pos += 1; // past '\'
        let (value, consumed) = process_escape(bytes, pos, true);
        pos += consumed;
        match value {
            0x100 => return false, // '\c' — stop
            0x101 => {
                out.push('\\');
                if consumed == 0 && pos < bytes.len() {
                    out.push(bytes[pos] as char);
                    pos += 1;
                }
            }
            v => out.push(v as u8 as char),
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Integer argument parsing
// ---------------------------------------------------------------------------

/// Parse an argument string as a signed integer per POSIX printf rules:
///   - Leading '+'/'-' allowed.
///   - Leading '0' → octal.
///   - Leading '0x'/'0X' → hexadecimal.
///   - Leading single-quote or double-quote → numeric value of next byte.
/// Returns (value, had_error).
fn parse_integer(s: &str) -> (i64, bool) {
    let s = s.trim();
    if s.is_empty() {
        write_diagnostic(s, "expected numeric value");
        return (0, true);
    }
    let bytes = s.as_bytes();
    // Single-quote or double-quote: value of the following character.
    if bytes[0] == b'\'' || bytes[0] == b'"' {
        if bytes.len() < 2 {
            write_diagnostic(s, "expected numeric value");
            return (0, true);
        }
        let value = bytes[1] as i64;
        if bytes.len() > 2 {
            write_diagnostic(s, "not completely converted");
            return (value, true);
        }
        return (value, false);
    }
    // Sign.
    let (negative, rest) = if bytes[0] == b'-' {
        (true, &bytes[1..])
    } else if bytes[0] == b'+' {
        (false, &bytes[1..])
    } else {
        (false, bytes)
    };
    if rest.is_empty() {
        write_diagnostic(s, "expected numeric value");
        return (0, true);
    }
    // Base detection.
    let (base, digits) = if rest.len() >= 2 && rest[0] == b'0'
        && (rest[1] == b'x' || rest[1] == b'X') {
        (16u64, &rest[2..])
    } else if rest[0] == b'0' && rest.len() > 1 {
        (8u64, &rest[1..])
    } else {
        (10u64, rest)
    };
    if digits.is_empty() && base != 8 {
        write_diagnostic(s, "expected numeric value");
        return (0, true);
    }
    let mut value: u64 = 0;
    let mut overflow = false;
    let mut consumed = 0usize;
    for &b in digits {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u64,
            b'a'..=b'f' if base == 16 => (b - b'a' + 10) as u64,
            b'A'..=b'F' if base == 16 => (b - b'A' + 10) as u64,
            _ => break,
        };
        if digit >= base { break; }
        value = match value.checked_mul(base).and_then(|v| v.checked_add(digit)) {
            Some(v) => v,
            None => { overflow = true; break; }
        };
        consumed += 1;
    }
    let leftover = consumed < digits.len();
    let result = if negative {
        (value as i64).wrapping_neg()
    } else {
        value as i64
    };
    if overflow {
        let clamped = if negative { i64::MIN } else { i64::MAX };
        write_diagnostic(s, "arithmetic overflow");
        return (clamped, true);
    }
    if leftover {
        write_diagnostic(s, "not completely converted");
        return (result, true);
    }
    (result, false)
}

fn write_diagnostic(arg: &str, reason: &str) {
    write_stderr("printf: \"");
    write_stderr(arg);
    write_stderr("\" ");
    write_stderr(reason);
    write_stderr("\n");
}

// ---------------------------------------------------------------------------
// Integer formatting
// ---------------------------------------------------------------------------

/// Format a u64 into `buf` in the given base. Returns start index.
fn format_uint_into(mut value: u64, base: u64, upper: bool, buf: &mut [u8; 66]) -> usize {
    if value == 0 {
        buf[65] = b'0';
        return 65;
    }
    let digits_lower = b"0123456789abcdef";
    let digits_upper = b"0123456789ABCDEF";
    let digits = if upper { digits_upper } else { digits_lower };
    let mut pos = 66usize;
    while value > 0 {
        pos -= 1;
        buf[pos] = digits[(value % base) as usize];
        value /= base;
    }
    pos
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn push_padded(
    out: &mut String,
    s: &str,
    width: usize,
    left_align: bool,
    pad_char: char,
) {
    let len = s.len();
    if !left_align && len < width {
        for _ in 0..(width - len) { out.push(pad_char); }
    }
    out.push_str(s);
    if left_align && len < width {
        for _ in 0..(width - len) { out.push(' '); }
    }
}

// ---------------------------------------------------------------------------
// Conversion spec parser
// ---------------------------------------------------------------------------

struct ConvSpec {
    /// Numbered argument (1-based). 0 = use sequential.
    numbered: usize,
    left_align:   bool,
    zero_pad:     bool,
    force_sign:   bool,
    space_sign:   bool,
    width:        usize,
    precision:    Option<usize>,
    specifier:    u8,
    /// Number of bytes consumed from the format string (past the '%').
    consumed:     usize,
}

/// Parse a conversion spec starting just after the '%'. Returns None if invalid.
fn parse_conv_spec(bytes: &[u8]) -> Option<ConvSpec> {
    let mut pos = 0usize;

    // Peek for numbered argument: digits followed by '$'.
    let mut numbered = 0usize;
    let save = pos;
    while pos < bytes.len() && bytes[pos] >= b'0' && bytes[pos] <= b'9' {
        numbered = numbered * 10 + (bytes[pos] - b'0') as usize;
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'$' && numbered > 0 {
        pos += 1; // consume '$'
    } else {
        // Not a numbered spec — reset.
        numbered = 0;
        pos = save;
    }

    // Flags.
    let mut left_align = false;
    let mut zero_pad   = false;
    let mut force_sign = false;
    let mut space_sign = false;
    loop {
        if pos >= bytes.len() { return None; }
        match bytes[pos] {
            b'-' => { left_align = true; pos += 1; }
            b'0' => { zero_pad   = true; pos += 1; }
            b'+' => { force_sign = true; pos += 1; }
            b' ' => { space_sign = true; pos += 1; }
            _    => break,
        }
    }
    if left_align { zero_pad = false; } // '-' overrides '0'

    // Width.
    let mut width = 0usize;
    while pos < bytes.len() && bytes[pos] >= b'0' && bytes[pos] <= b'9' {
        width = width * 10 + (bytes[pos] - b'0') as usize;
        pos += 1;
    }

    // Precision.
    let precision = if pos < bytes.len() && bytes[pos] == b'.' {
        pos += 1;
        let mut prec = 0usize;
        while pos < bytes.len() && bytes[pos] >= b'0' && bytes[pos] <= b'9' {
            prec = prec * 10 + (bytes[pos] - b'0') as usize;
            pos += 1;
        }
        Some(prec)
    } else {
        None
    };

    // Specifier.
    if pos >= bytes.len() { return None; }
    let specifier = bytes[pos];
    pos += 1;

    Some(ConvSpec { numbered, left_align, zero_pad, force_sign, space_sign,
                    width, precision, specifier, consumed: pos })
}

// ---------------------------------------------------------------------------
// Main formatting loop
// ---------------------------------------------------------------------------

/// Process the format string once against `args` starting at `arg_index`.
/// Returns (new_arg_index, stop_printing).
/// `stop_printing` = true if '\c' was encountered in a %b argument.
fn format_once(
    format: &str,
    format_args: &[String],
    sequential_arg: &mut usize,
    out: &mut String,
    had_error: &mut bool,
) -> bool {
    let bytes = format.as_bytes();
    let mut pos = 0usize;

    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos += 1;
            let (value, consumed) = process_escape(bytes, pos, false);
            pos += consumed;
            if value == 0x101 {
                out.push('\\');
                if consumed == 0 && pos < bytes.len() {
                    out.push(bytes[pos] as char);
                    pos += 1;
                }
            } else {
                out.push(value as u8 as char);
            }
            continue;
        }

        if bytes[pos] != b'%' {
            out.push(bytes[pos] as char);
            pos += 1;
            continue;
        }
        pos += 1; // past '%'

        // Handle '%%' immediately.
        if pos < bytes.len() && bytes[pos] == b'%' {
            out.push('%');
            pos += 1;
            continue;
        }

        // Parse conversion spec.
        let spec = match parse_conv_spec(&bytes[pos..]) {
            Some(s) => s,
            None => {
                // Invalid — output '%' literally and continue.
                out.push('%');
                continue;
            }
        };
        pos += spec.consumed;

        // Resolve argument.
        let arg_idx = if spec.numbered > 0 {
            spec.numbered - 1 // convert to 0-based
        } else {
            let idx = *sequential_arg;
            if spec.specifier != b'%' {
                *sequential_arg += 1;
            }
            idx
        };

        // Get the argument string (or empty/zero placeholder if missing).
        let arg_str: &str = format_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");

        match spec.specifier {
            b's' => {
                let s = if let Some(prec) = spec.precision {
                    // Precision limits the number of bytes written.
                    let end = arg_str.len().min(prec);
                    // Snap to a valid char boundary.
                    let end = (0..=end).rev()
                        .find(|&i| arg_str.is_char_boundary(i))
                        .unwrap_or(0);
                    &arg_str[..end]
                } else {
                    arg_str
                };
                push_padded(out, s, spec.width, spec.left_align, ' ');
            }
            b'b' => {
                let mut expanded = String::new();
                let stop = !expand_b_escapes(arg_str, &mut expanded);
                let s = if let Some(prec) = spec.precision {
                    let end = expanded.len().min(prec);
                    let end = (0..=end).rev()
                        .find(|&i| expanded.is_char_boundary(i))
                        .unwrap_or(0);
                    String::from(&expanded[..end])
                } else {
                    expanded.clone()
                };
                push_padded(out, &s, spec.width, spec.left_align, ' ');
                if stop { return true; }
            }
            b'c' => {
                let ch = arg_str.bytes().next().unwrap_or(0);
                let ch_buf = [ch];
                let s = core::str::from_utf8(&ch_buf).unwrap_or("");
                push_padded(out, s, spec.width, spec.left_align, ' ');
            }
            b'd' | b'i' => {
                let (value, error) = parse_integer(arg_str);
                if error { *had_error = true; }
                let mut buf = [0u8; 66];
                let abs_val = if value < 0 {
                    (value as u64).wrapping_neg()
                } else {
                    value as u64
                };
                let start = format_uint_into(abs_val, 10, false, &mut buf);
                let digits = core::str::from_utf8(&buf[start..]).unwrap_or("0");
                // Apply precision (minimum digits).
                let min_digits = spec.precision.unwrap_or(1).max(digits.len());
                // Build sign + zero-extended digits.
                let mut num_str = String::new();
                let sign = if value < 0 { Some('-') }
                           else if spec.force_sign { Some('+') }
                           else if spec.space_sign { Some(' ') }
                           else { None };
                if let Some(s) = sign { num_str.push(s); }
                for _ in digits.len()..min_digits { num_str.push('0'); }
                num_str.push_str(digits);
                let pad = if spec.zero_pad && spec.precision.is_none() { '0' } else { ' ' };
                push_padded(out, &num_str, spec.width, spec.left_align, pad);
            }
            b'u' => {
                let (value, error) = parse_integer(arg_str);
                if error { *had_error = true; }
                let uval = value as u64;
                let mut buf = [0u8; 66];
                let start = format_uint_into(uval, 10, false, &mut buf);
                let digits = core::str::from_utf8(&buf[start..]).unwrap_or("0");
                let min_digits = spec.precision.unwrap_or(1).max(digits.len());
                let mut num_str = String::new();
                for _ in digits.len()..min_digits { num_str.push('0'); }
                num_str.push_str(digits);
                let pad = if spec.zero_pad && spec.precision.is_none() { '0' } else { ' ' };
                push_padded(out, &num_str, spec.width, spec.left_align, pad);
            }
            b'o' => {
                let (value, error) = parse_integer(arg_str);
                if error { *had_error = true; }
                let uval = value as u64;
                let mut buf = [0u8; 66];
                let start = format_uint_into(uval, 8, false, &mut buf);
                let digits = core::str::from_utf8(&buf[start..]).unwrap_or("0");
                let min_digits = spec.precision.unwrap_or(1).max(digits.len());
                let mut num_str = String::new();
                for _ in digits.len()..min_digits { num_str.push('0'); }
                num_str.push_str(digits);
                let pad = if spec.zero_pad && spec.precision.is_none() { '0' } else { ' ' };
                push_padded(out, &num_str, spec.width, spec.left_align, pad);
            }
            b'x' | b'X' => {
                let upper = spec.specifier == b'X';
                let (value, error) = parse_integer(arg_str);
                if error { *had_error = true; }
                let uval = value as u64;
                let mut buf = [0u8; 66];
                let start = format_uint_into(uval, 16, upper, &mut buf);
                let digits = core::str::from_utf8(&buf[start..]).unwrap_or("0");
                let min_digits = spec.precision.unwrap_or(1).max(digits.len());
                let mut num_str = String::new();
                for _ in digits.len()..min_digits { num_str.push('0'); }
                num_str.push_str(digits);
                let pad = if spec.zero_pad && spec.precision.is_none() { '0' } else { ' ' };
                push_padded(out, &num_str, spec.width, spec.left_align, pad);
            }
            _ => {
                // Unknown specifier — output '%' + the char.
                out.push('%');
                out.push(spec.specifier as char);
            }
        }
    }

    false // no '\c' encountered
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    if arguments.len() < 2 {
        write_stderr("usage: printf format [argument...]\n");
        raw::raw_exit(1);
    }

    let format = arguments[1].as_str();

    // Collect argument operands as owned Strings for indexed access.
    let format_args: Vec<String> = arguments[2..].iter()
        .map(|s| s.clone())
        .collect();

    let mut had_error = false;
    let mut output = String::new();

    if format_args.is_empty() {
        // No arguments: process format once (escape sequences only).
        expand_format_escapes(format, &mut output);
    } else {
        // Reuse format until all arguments are consumed.
        let mut sequential_arg = 0usize;
        loop {
            let stop = format_once(
                format,
                &format_args,
                &mut sequential_arg,
                &mut output,
                &mut had_error,
            );
            if stop { break; }
            // Stop reusing when all sequential arguments have been consumed
            // and there are no numbered-argument specs that could consume more.
            if sequential_arg >= format_args.len() { break; }
        }
    }

    write_stdout(&output);
    raw::raw_exit(if had_error { 1 } else { 0 });
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
