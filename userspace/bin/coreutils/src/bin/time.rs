#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout, write_stderr};

// CLOCK_MONOTONIC
const CLOCK_MONOTONIC: i32 = 1;

// RUSAGE_CHILDREN: resource usage of waited-for children.
const RUSAGE_CHILDREN: i32 = -1i32;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let _ = argc;

    let arguments = args();
    if arguments.len() < 2 {
        write_stderr("usage: time <command> [args...]\n");
        raw::raw_exit(1);
    }

    // Record wall clock start.
    let mut start_ts = [0u64; 2];
    raw::raw_clock_gettime(CLOCK_MONOTONIC, start_ts.as_mut_ptr());

    // Fork and exec the child.
    let child_pid = raw::raw_fork();
    if child_pid < 0 {
        write_stderr("time: fork failed\n");
        raw::raw_exit(1);
    }

    if child_pid == 0 {
        // Child: exec the requested command.
        let command = &arguments[1];
        // Build flat argv: "cmd\0arg1\0arg2\0"
        let mut flat: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        for argument in &arguments[1..] {
            flat.extend_from_slice(argument.as_bytes());
            flat.push(0);
        }
        raw::raw_exec(
            command.as_ptr(),
            command.len(),
            flat.as_ptr(),
            flat.len(),
        );
        // exec failed
        write_stderr("time: exec failed: ");
        write_stderr(command.as_str());
        write_stderr("\n");
        raw::raw_exit(127);
    }

    // Parent: wait for child.
    let mut exit_status: i32 = 0;
    raw::raw_wait(child_pid as i32, &mut exit_status as *mut i32);

    // Record wall clock end.
    let mut end_ts = [0u64; 2];
    raw::raw_clock_gettime(CLOCK_MONOTONIC, end_ts.as_mut_ptr());

    // Compute elapsed wall time.
    let wall_seconds = end_ts[0].saturating_sub(start_ts[0]);
    let wall_nanos = if end_ts[1] >= start_ts[1] {
        end_ts[1] - start_ts[1]
    } else {
        // Borrow 1 second.
        1_000_000_000u64 + end_ts[1] - start_ts[1]
    };
    let wall_ms = wall_seconds * 1000 + wall_nanos / 1_000_000;

    // Get CPU time from kernel (user + sys via getrusage RUSAGE_CHILDREN).
    // rusage layout: ru_utime(sec,usec), ru_stime(sec,usec) at offsets 0-3 (u64 each).
    let mut rusage = [0u64; 4];
    raw::raw_getrusage(RUSAGE_CHILDREN, rusage.as_mut_ptr() as *mut u8);
    let user_sec  = rusage[0];
    let user_usec = rusage[1];
    let sys_sec   = rusage[2];
    let sys_usec  = rusage[3];

    write_stderr("\nreal\t");
    write_time_ms(wall_ms);
    write_stderr("\nuser\t");
    write_time_sec_usec(user_sec, user_usec);
    write_stderr("\nsys\t");
    write_time_sec_usec(sys_sec, sys_usec);
    write_stderr("\n");

    let exit_code = (exit_status >> 8) & 0xFF;
    raw::raw_exit(exit_code)
}

fn write_time_ms(total_ms: u64) {
    let seconds = total_ms / 1000;
    let millis  = total_ms % 1000;
    write_u64_decimal(seconds);
    write_stderr(".");
    // 3 decimal places for milliseconds.
    write_padded_u64(millis, 3);
    write_stderr("s");
}

fn write_time_sec_usec(seconds: u64, microseconds: u64) {
    let millis = (seconds * 1000) + (microseconds / 1000);
    write_time_ms(millis);
}

fn write_u64_decimal(value: u64) {
    let mut buf = [0u8; 20];
    let mut cursor = 20usize;
    let mut v = value;
    if v == 0 {
        cursor -= 1;
        buf[cursor] = b'0';
    } else {
        while v > 0 {
            cursor -= 1;
            buf[cursor] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    }
    if let Ok(s) = core::str::from_utf8(&buf[cursor..]) {
        write_stderr(s);
    }
}

fn write_padded_u64(value: u64, width: usize) {
    let mut buf = [b'0'; 20];
    let mut cursor = 20usize;
    let mut v = value;
    if v == 0 {
        cursor -= 1;
        buf[cursor] = b'0';
    } else {
        while v > 0 {
            cursor -= 1;
            buf[cursor] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    }
    let digits = 20 - cursor;
    if width > digits {
        let pad_count = width - digits;
        let pad_buf = [b'0'; 20];
        if let Ok(s) = core::str::from_utf8(&pad_buf[..pad_count]) {
            write_stderr(s);
        }
    }
    if let Ok(s) = core::str::from_utf8(&buf[cursor..]) {
        write_stderr(s);
    }
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
