#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, write_stdout, die};
use alloc::string::String;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    if arguments.len() < 2 {
        die("usage: printf <format> [args...]");
    }
    let format = &arguments[1];
    let format_args = &arguments[2..];
    let mut argument_index = 0;
    let mut output = String::new();
    let mut chars = format.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '%' {
            output.push(character);
            continue;
        }
        match chars.next() {
            None => break,
            Some('%') => output.push('%'),
            Some('s') => {
                if let Some(arg) = format_args.get(argument_index) {
                    output.push_str(arg);
                    argument_index += 1;
                }
            }
            Some('d') | Some('i') => {
                if let Some(arg) = format_args.get(argument_index) {
                    output.push_str(arg); // Already a decimal string from argv.
                    argument_index += 1;
                }
            }
            Some('u') => {
                if let Some(arg) = format_args.get(argument_index) {
                    output.push_str(arg);
                    argument_index += 1;
                }
            }
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some(other) => {
                output.push('%');
                output.push(other);
            }
        }
    }
    write_stdout(&output);
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
