#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, write_stderr, die};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    if arguments.len() < 3 {
        die("usage: mv <source> <destination>");
    }
    let source = &arguments[1];
    let destination = &arguments[2];

    let data = match read_file_or_stdin(Some(source)) {
        Ok(d) => d,
        Err(_) => {
            write_stderr("mv: cannot read '");
            write_stderr(source);
            write_stderr("'\n");
            raw::raw_exit(1)
        }
    };

    let destination_fd = raw::raw_creat(destination.as_ptr(), destination.len());
    if destination_fd < 0 {
        write_stderr("mv: cannot create '");
        write_stderr(destination);
        write_stderr("'\n");
        raw::raw_exit(1);
    }
    raw::raw_write(destination_fd as i32, data.as_ptr(), data.len());
    raw::raw_close(destination_fd as i32);

    // Remove the source after successful copy.
    raw::raw_unlink(source.as_ptr(), source.len());
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
