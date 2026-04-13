#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::time::DateTime;
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();

    // Optional format string: date +%Y-%m-%d etc. (minimal support).
    let output = if let Some(format_arg) = arguments.get(1) {
        let format_str = format_arg.as_str();
        if format_str.starts_with('+') {
            let now = DateTime::now();
            apply_format(&now, &format_str[1..])
        } else {
            write_stderr("date: invalid option: ");
            write_stderr(format_str);
            write_stderr("\n");
            raw::raw_exit(1);
        }
    } else {
        DateTime::now().format_posix_date()
    };

    write_stdout(output.as_str());
    write_stdout("\n");
    raw::raw_exit(0)
}

/// Apply a minimal subset of `date`-style format directives.
///
/// Supported: %Y %m %d %H %M %S %A %a %B %b %Z %n %t %%
fn apply_format(datetime: &DateTime, format: &str) -> alloc::string::String {
    let mut result = alloc::string::String::new();
    let bytes = format.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 1 < bytes.len() {
            index += 1;
            match bytes[index] {
                b'Y' => push_i32_padded(&mut result, datetime.year, 4),
                b'm' => push_u8_padded(&mut result, datetime.month),
                b'd' => push_u8_padded(&mut result, datetime.day),
                b'H' => push_u8_padded(&mut result, datetime.hour),
                b'M' => push_u8_padded(&mut result, datetime.minute),
                b'S' => push_u8_padded(&mut result, datetime.second),
                b'A' => result.push_str(datetime.weekday_name_short()),
                b'a' => result.push_str(datetime.weekday_name_short()),
                b'B' => result.push_str(datetime.month_name_short()),
                b'b' => result.push_str(datetime.month_name_short()),
                b'Z' => result.push_str("UTC"),
                b'n' => result.push('\n'),
                b't' => result.push('\t'),
                b'%' => result.push('%'),
                other => {
                    result.push('%');
                    result.push(other as char);
                }
            }
        } else {
            result.push(bytes[index] as char);
        }
        index += 1;
    }
    result
}

fn push_u8_padded(s: &mut alloc::string::String, value: u8) {
    if value < 10 { s.push('0'); }
    let mut buf = [0u8; 3];
    let mut cursor = 3usize;
    let mut v = value;
    if v == 0 { cursor -= 1; buf[cursor] = b'0'; }
    else { while v > 0 { cursor -= 1; buf[cursor] = b'0' + v % 10; v /= 10; } }
    if let Ok(st) = core::str::from_utf8(&buf[cursor..]) { s.push_str(st); }
}

fn push_i32_padded(s: &mut alloc::string::String, value: i32, width: usize) {
    let mut buf = [0u8; 10];
    let mut cursor = 10usize;
    let mut v = if value < 0 { 0u32 } else { value as u32 };
    if v == 0 { cursor -= 1; buf[cursor] = b'0'; }
    else { while v > 0 { cursor -= 1; buf[cursor] = b'0' + (v % 10) as u8; v /= 10; } }
    let digits = 10 - cursor;
    for _ in digits..width { s.push('0'); }
    if let Ok(st) = core::str::from_utf8(&buf[cursor..]) { s.push_str(st); }
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
