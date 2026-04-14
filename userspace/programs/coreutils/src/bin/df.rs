// POSIX.1-2024 — df
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/df.html
//
// Write the amount of available space for mounted file systems.
//
// Options:
//   -k   Use 1024-byte units instead of the default 512-byte units.
//   -P   Produce output in the POSIX portable format (mandatory header + one
//        line per filesystem with exact "%s %d %d %d %d%% %s\n" layout).
//   -t   [XSI] Include total allocated-space figures in the output.
//
// POSIX -P output format:
//   Header (with -k):  "Filesystem 1024-blocks Used Available Capacity Mounted on\n"
//   Header (no -k):    "Filesystem 512-blocks  Used Available Capacity Mounted on\n"
//   Data line:         "%s %d %d %d %d%% %s\n"
//                       <fs-name> <total> <used> <free> <pct> <mountpoint>
//
// Space figures are in 512-byte units (or 1024-byte with -k), rounded up.
// Percentage is ceiling of used / (used + free) * 100.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_io::getmounts;
use coreutils::{args, write_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    // Parse options.
    let mut use_kilobyte_units = false;
    let mut use_posix_format   = false;
    let mut include_totals     = false;

    for argument in arguments[1..].iter() {
        match argument.as_str() {
            "-k" => use_kilobyte_units = true,
            "-P" => use_posix_format   = true,
            "-t" => include_totals     = true,
            "--" => break,
            arg if arg.starts_with('-') => {
                // Compound flags e.g. -kP
                for character in arg[1..].chars() {
                    match character {
                        'k' => use_kilobyte_units = true,
                        'P' => use_posix_format   = true,
                        't' => include_totals      = true,
                        other => {
                            write_stderr("df: invalid option -- '");
                            let mut byte_buf = [0u8; 4];
                            let encoded = other.encode_utf8(&mut byte_buf);
                            write_stderr(encoded);
                            write_stderr("'\n");
                            raw::raw_exit(1);
                        }
                    }
                }
            }
            _ => {} // file operand — not yet used to filter filesystems
        }
    }

    // -P and -t are mutually exclusive per POSIX synopsis: [-P|-t]
    if use_posix_format && include_totals {
        write_stderr("df: options -P and -t are mutually exclusive\n");
        raw::raw_exit(1);
    }

    let mounts = getmounts();

    if mounts.is_empty() {
        write_stderr("df: no mounted filesystems reported by kernel\n");
        raw::raw_exit(1);
    }

    if use_posix_format {
        // POSIX -P: strict format defined by the specification.
        if use_kilobyte_units {
            write_stdout("Filesystem         1024-blocks      Used Available Capacity Mounted on\n");
        } else {
            write_stdout("Filesystem          512-blocks      Used Available Capacity Mounted on\n");
        }

        for entry in &mounts {
            let display_source: &str = if !entry.source.is_empty() {
                &entry.source
            } else {
                &entry.fstype
            };

            // Convert from 512-byte blocks (kernel unit) to the requested unit.
            // Kernel always reports in 512-byte blocks; -k divides by 2.
            let (total, free) = if use_kilobyte_units {
                // Round up: (n + 1) / 2
                (
                    (entry.total_blocks + 1) / 2,
                    (entry.free_blocks  + 1) / 2,
                )
            } else {
                (entry.total_blocks, entry.free_blocks)
            };

            let used = total.saturating_sub(free);

            // Percentage: ceiling of used / (used + free) * 100.
            // POSIX: "rounded up to the next highest integer"
            let percentage = if total == 0 {
                0u64
            } else {
                (used * 100 + total - 1) / total
            };

            // POSIX format: "%s %d %d %d %d%% %s\n"
            write_stdout(display_source);
            write_stdout(" ");
            write_u64(total);
            write_stdout(" ");
            write_u64(used);
            write_stdout(" ");
            write_u64(free);
            write_stdout(" ");
            write_u64(percentage);
            write_stdout("% ");
            write_stdout(&entry.mountpoint);
            write_stdout("\n");
        }
    } else {
        // Default (non -P) format: implementation-defined per POSIX, but must
        // report at least filesystem name, available space, and (without -t)
        // number of free inodes. We match the conventional tabular layout.
        let unit_label = if use_kilobyte_units { "1024-blocks" } else { "512-blocks " };

        if include_totals {
            write_stdout("Filesystem           ");
            write_stdout(unit_label);
            write_stdout("        Used   Available  Capacity     Total  Mounted on\n");
        } else {
            write_stdout("Filesystem           ");
            write_stdout(unit_label);
            write_stdout("        Used   Available  Capacity  Mounted on\n");
        }

        for entry in &mounts {
            let display_source: &str = if !entry.source.is_empty() {
                &entry.source
            } else {
                &entry.fstype
            };

            let (total, free) = if use_kilobyte_units {
                ((entry.total_blocks + 1) / 2, (entry.free_blocks + 1) / 2)
            } else {
                (entry.total_blocks, entry.free_blocks)
            };

            let used = total.saturating_sub(free);
            let percentage = if total == 0 {
                0u64
            } else {
                (used * 100 + total - 1) / total
            };

            // Column 1: Filesystem (20 chars, left-aligned).
            write_stdout(display_source);
            let source_length = display_source.len();
            if source_length < 20 {
                pad_spaces(20 - source_length);
            } else {
                write_stdout("\n");
                pad_spaces(20);
            }

            if total > 0 {
                print_u64_right(total, 12);
                print_u64_right(used, 12);
                print_u64_right(free, 12);

                // Capacity: right-aligned in 9 chars, then '%', then spaces.
                let digit_count = decimal_digit_count(percentage);
                pad_spaces(9usize.saturating_sub(digit_count));
                write_u64(percentage);
                write_stdout("%  ");

                if include_totals {
                    print_u64_right(total, 10);
                    write_stdout("  ");
                }
            } else {
                if include_totals {
                    write_stdout("           -           -           -         -           -  ");
                } else {
                    write_stdout("           -           -           -         -  ");
                }
            }

            write_stdout(&entry.mountpoint);
            write_stdout("\n");
        }
    }

    raw::raw_exit(0);
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Write the decimal representation of `value` to stdout.
fn write_u64(value: u64) {
    let mut buf = [0u8; 20];
    let start = u64_into_buf(value, &mut buf);
    let slice = unsafe { core::str::from_utf8_unchecked(&buf[start..]) };
    write_stdout(slice);
}

/// Print `value` right-aligned in a field of `width` characters.
fn print_u64_right(value: u64, width: usize) {
    let digit_count = decimal_digit_count(value);
    pad_spaces(width.saturating_sub(digit_count));
    write_u64(value);
}

/// Write `count` ASCII space characters to stdout.
fn pad_spaces(count: usize) {
    const SPACES: &str = "                                ";
    let mut remaining = count;
    while remaining > 0 {
        let chunk = remaining.min(32);
        write_stdout(&SPACES[..chunk]);
        remaining -= chunk;
    }
}

/// Return the number of decimal digits needed to represent `value`.
fn decimal_digit_count(value: u64) -> usize {
    if value == 0 { return 1; }
    let mut n = value;
    let mut count = 0usize;
    while n > 0 { n /= 10; count += 1; }
    count
}

/// Write `value` as decimal into `buf`, returning the start index of the digits.
fn u64_into_buf(value: u64, buf: &mut [u8; 20]) -> usize {
    if value == 0 {
        buf[19] = b'0';
        return 19;
    }
    let mut n = value;
    let mut pos = 20usize;
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    pos
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
