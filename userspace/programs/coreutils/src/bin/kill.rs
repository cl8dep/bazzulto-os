#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();

    // Parse: kill [-<signal>] <pid>...
    // Default signal: SIGTERM (15).
    let mut signal_number: i32 = 15;
    let mut pid_start = 1usize;

    if let Some(first) = arguments.get(1) {
        let first_str = first.as_str();
        if first_str.starts_with('-') {
            let signal_str = &first_str[1..];
            // Accept numeric signal or signal name (SIG-less prefix).
            signal_number = match parse_signal(signal_str) {
                Some(n) => n,
                None => {
                    write_stderr("kill: invalid signal: ");
                    write_stderr(signal_str);
                    write_stderr("\n");
                    raw::raw_exit(1);
                }
            };
            pid_start = 2;
        }
    }

    if arguments.len() <= pid_start {
        write_stderr("kill: missing PID operand\n");
        write_stderr("usage: kill [-<signal>] <pid>...\n");
        raw::raw_exit(1);
    }

    let mut had_error = false;
    for pid_str in &arguments[pid_start..] {
        let pid = match parse_i32(pid_str.as_str()) {
            Some(p) => p,
            None => {
                write_stderr("kill: invalid PID: ");
                write_stderr(pid_str.as_str());
                write_stderr("\n");
                had_error = true;
                continue;
            }
        };
        let result = raw::raw_kill(pid, signal_number);
        if result < 0 {
            write_stderr("kill: failed to send signal to PID ");
            write_stderr(pid_str.as_str());
            write_stderr("\n");
            had_error = true;
        }
    }

    raw::raw_exit(if had_error { 1 } else { 0 })
}

fn parse_signal(name: &str) -> Option<i32> {
    // Numeric signal.
    if let Some(n) = parse_i32(name) {
        return Some(n);
    }
    // Named signal (without "SIG" prefix or with it).
    let canonical = if name.len() > 3 && &name[..3] == "SIG" {
        &name[3..]
    } else {
        name
    };
    let signal = match canonical {
        "HUP"  => 1,
        "INT"  => 2,
        "QUIT" => 3,
        "ILL"  => 4,
        "TRAP" => 5,
        "ABRT" => 6,
        "BUS"  => 7,
        "FPE"  => 8,
        "KILL" => 9,
        "USR1" => 10,
        "SEGV" => 11,
        "USR2" => 12,
        "PIPE" => 13,
        "ALRM" => 14,
        "TERM" => 15,
        "CHLD" => 17,
        "CONT" => 18,
        "STOP" => 19,
        "TSTP" => 20,
        "TTIN" => 21,
        "TTOU" => 22,
        _ => return None,
    };
    Some(signal)
}

fn parse_i32(s: &str) -> Option<i32> {
    let (negative, digits) = if s.starts_with('-') {
        (true, &s[1..])
    } else {
        (false, s)
    };
    if digits.is_empty() {
        return None;
    }
    let mut result: i32 = 0;
    for byte in digits.bytes() {
        if byte < b'0' || byte > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((byte - b'0') as i32)?;
    }
    Some(if negative { -result } else { result })
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
