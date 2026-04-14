#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::time::{DateTime, DateTimeLocal};
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();

    // Parse flags and format argument.
    // Usage: date [-u] [+FORMAT]
    let mut force_utc  = false;
    let mut format_arg: Option<&str> = None;

    let mut arg_index = 1usize;
    while arg_index < arguments.len() {
        let arg = arguments[arg_index].as_str();
        if arg == "-u" || arg == "--utc" || arg == "--universal" {
            force_utc = true;
        } else if arg.starts_with('+') {
            format_arg = Some(arg);
        } else {
            write_stderr("date: invalid option: ");
            write_stderr(arg);
            write_stderr("\n");
            raw::raw_exit(1);
        }
        arg_index += 1;
    }

    let output = if force_utc {
        // Always UTC regardless of $TZ.
        let now = DateTime::now();
        if let Some(fmt) = format_arg {
            apply_format_utc(&now, &fmt[1..])
        } else {
            now.format_posix_date()
        }
    } else {
        // Local time via $TZ / /etc/localtime.
        let now_local = DateTimeLocal::now();
        if let Some(fmt) = format_arg {
            now_local.format(&fmt[1..])
        } else {
            now_local.format_posix_date()
        }
    };

    write_stdout(output.as_str());
    write_stdout("\n");
    raw::raw_exit(0)
}

/// Apply a strftime-style format string to a UTC `DateTime`.
///
/// Delegates entirely to `DateTime::format`; `%Z` produces `"UTC"` and
/// `%z` produces `"+0000"`.
fn apply_format_utc(datetime: &DateTime, format: &str) -> alloc::string::String {
    datetime.format(format)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
