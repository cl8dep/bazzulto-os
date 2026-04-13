#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::environment::Environment;
use coreutils::write_stdout;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let exit_code = main();
    raw::raw_exit(exit_code)
}

fn main() -> i32 {
    let arguments = Environment::args();

    // `env` without arguments: print all variables in `KEY=VALUE\n` format.
    // `env KEY=VALUE ... command [args]`: set variables then exec command.
    // Simplified implementation: only the no-argument form is supported here.
    //
    // Reference: POSIX.1-2017 env(1).

    if arguments.len() <= 1 {
        // Print the full environment, one `KEY=VALUE` per line.
        let env = Environment::all();
        for (key, value) in &env {
            write_stdout(key.as_str());
            write_stdout("=");
            write_stdout(value.as_str());
            write_stdout("\n");
        }
        return 0;
    }

    // Argument parsing: consume leading `KEY=VALUE` assignments, then treat
    // the remainder as a command to execute (not yet implemented).
    let mut first_command_index = 1usize;
    for arg in &arguments[1..] {
        if arg.contains('=') {
            let eq = arg.find('=').unwrap();
            Environment::set(&arg[..eq], &arg[eq + 1..]);
            first_command_index += 1;
        } else {
            break;
        }
    }

    if first_command_index >= arguments.len() {
        // Only assignments, no command: print the resulting environment.
        let env = Environment::all();
        for (key, value) in &env {
            write_stdout(key.as_str());
            write_stdout("=");
            write_stdout(value.as_str());
            write_stdout("\n");
        }
        return 0;
    }

    // Command execution not yet supported (requires execve from userspace).
    write_stdout("env: exec not yet supported\n");
    1
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
