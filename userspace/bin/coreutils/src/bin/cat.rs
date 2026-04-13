#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use bazzulto_system::raw;
use coreutils::{args, read_file_or_stdin, write_bytes_stdout, write_stderr};

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();
    let paths: alloc::vec::Vec<&str> = arguments[1..].iter().map(|s| s.as_str()).collect();

    if paths.is_empty() {
        let data = read_file_or_stdin(None).unwrap_or_default();
        write_bytes_stdout(&data);
    } else {
        for path in &paths {
            match read_file_or_stdin(Some(path)) {
                Ok(data) => write_bytes_stdout(&data),
                Err(_) => {
                    write_stderr("cat: ");
                    write_stderr(path);
                    write_stderr(": No such file or directory\n");
                    raw::raw_exit(1);
                }
            }
        }
    }
    raw::raw_exit(0)
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
