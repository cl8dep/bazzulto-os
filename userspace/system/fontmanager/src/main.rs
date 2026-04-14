//! bzfontd — Bazzulto font management CLI.
//!
//! # Usage
//!
//! ```
//! bzfontd list                    # list installed fonts in /usr/share/fonts/
//! bzfontd install /path/to/font.ttf   # install a TTF into /usr/share/fonts/
//! bzfontd verify  FontName        # verify the font parses and rasterizes
//! bzfontd help                    # print usage
//! ```
//!
//! The font library itself lives in Bazzulto.Display (bazzulto-display crate).
//! bzfontd is the administrative interface — it does not run as a daemon.

#![no_std]
#![no_main]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use bazzulto_system::raw;
use bazzulto_display::font_manager::{FontManager, SYSTEM_FONT_DIRECTORIES, read_file};
use bazzulto_io::directory::Directory;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let exit_code = main();
    raw::raw_exit(exit_code)
}

fn main() -> i32 {
    // Read argv from stdin: bzfontd reads one line containing the arguments.
    // Convention: argv[0] is the command, rest are arguments.
    // The kernel's exec passes args via the ELF aux vector — for now we read
    // a single line from stdin as the argument string.
    let args = read_args();
    let subcommand = args.first().map(|string| string.as_str()).unwrap_or("help");

    match subcommand {
        "list"    => command_list(),
        "install" => {
            if let Some(source_path) = args.get(1) {
                command_install(source_path)
            } else {
                write_stderr("bzfontd: install requires a path argument\n");
                write_stderr("usage: bzfontd install /path/to/font.ttf\n");
                1
            }
        }
        "verify" => {
            if let Some(font_name) = args.get(1) {
                command_verify(font_name)
            } else {
                write_stderr("bzfontd: verify requires a font name argument\n");
                write_stderr("usage: bzfontd verify FontName\n");
                1
            }
        }
        "help" | "--help" | "-h" => {
            print_usage();
            0
        }
        unknown => {
            write_stderr("bzfontd: unknown subcommand '");
            write_stderr(unknown);
            write_stderr("'\n");
            print_usage();
            1
        }
    }
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// List all .ttf files installed in the system font directories.
fn command_list() -> i32 {
    let mut found_any = false;

    for directory in SYSTEM_FONT_DIRECTORIES {
        let entries = Directory::list_with_suffix(".ttf");
        for entry_path in &entries {
            if entry_path.starts_with(directory) {
                write_stdout(entry_path);
                write_stdout("\n");
                found_any = true;
            }
        }
    }

    if !found_any {
        write_stdout("bzfontd: no fonts installed\n");
        write_stdout("  install with: bzfontd install /path/to/font.ttf\n");
    }

    0
}

/// Install a TTF font from `source_path` into /usr/share/fonts/.
///
/// Copies the file byte-for-byte. The destination name is derived from the
/// last path component of `source_path` (e.g. `/mnt/Inter.ttf` → `Inter.ttf`).
fn command_install(source_path: &str) -> i32 {
    // Derive the font file name from the source path.
    let file_name = source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path);

    if !file_name.ends_with(".ttf") {
        write_stderr("bzfontd: only TTF fonts are supported\n");
        return 1;
    }

    // Read source file.
    let font_data = match read_file(source_path) {
        Ok(data) => data,
        Err(_) => {
            write_stderr("bzfontd: cannot read '");
            write_stderr(source_path);
            write_stderr("'\n");
            return 1;
        }
    };

    // Verify it parses before installing.
    if fontdue::Font::from_bytes(font_data.as_slice(), fontdue::FontSettings::default()).is_err() {
        write_stderr("bzfontd: '");
        write_stderr(source_path);
        write_stderr("' is not a valid TTF font\n");
        return 1;
    }

    // Build destination path: /usr/share/fonts/<filename>
    let mut destination_path = String::from("/usr/share/fonts/");
    destination_path.push_str(file_name);

    // Write to destination.
    let mut destination_path_buf = [0u8; 512];
    let destination_path_len = destination_path.len().min(511);
    destination_path_buf[..destination_path_len].copy_from_slice(&destination_path.as_bytes()[..destination_path_len]);
    let destination_fd = raw::raw_creat(destination_path_buf.as_ptr(), 0o644);
    if destination_fd < 0 {
        write_stderr("bzfontd: cannot create '");
        write_stderr(&destination_path);
        write_stderr("' — is /usr/share/fonts/ mounted?\n");
        return 1;
    }
    let destination_fd = destination_fd as i32;

    let bytes_written = raw::raw_write(destination_fd, font_data.as_ptr(), font_data.len());
    raw::raw_close(destination_fd);

    if bytes_written < 0 || bytes_written as usize != font_data.len() {
        write_stderr("bzfontd: write failed for '");
        write_stderr(&destination_path);
        write_stderr("'\n");
        return 1;
    }

    write_stdout("bzfontd: installed ");
    write_stdout(file_name);
    write_stdout(" → ");
    write_stdout(&destination_path);
    write_stdout("\n");

    0
}

/// Verify that a named font can be loaded and rasterizes the character 'A'.
fn command_verify(font_name: &str) -> i32 {
    let mut font_manager = FontManager::new();

    match font_manager.load_font(font_name, bazzulto_display::font_manager::FontVariant::default()) {
        Err(_) => {
            write_stderr("bzfontd: font '");
            write_stderr(font_name);
            write_stderr("' not found in any font directory\n");
            return 1;
        }
        Ok(font_id) => {
            match font_manager.rasterize(font_id, 'A', 16.0) {
                None => {
                    write_stderr("bzfontd: font loaded but failed to rasterize 'A' at 16pt\n");
                    return 1;
                }
                Some(bitmap) => {
                    write_stdout("bzfontd: '");
                    write_stdout(font_name);
                    write_stdout("' ok — 'A' at 16pt: ");
                    write_u32(bitmap.width);
                    write_stdout("×");
                    write_u32(bitmap.height);
                    write_stdout(" px\n");
                }
            }
        }
    }

    0
}

fn print_usage() {
    write_stdout("bzfontd — Bazzulto font manager\n");
    write_stdout("\n");
    write_stdout("usage:\n");
    write_stdout("  bzfontd list                      list installed fonts\n");
    write_stdout("  bzfontd install <path.ttf>        install a font\n");
    write_stdout("  bzfontd verify  <FontName>        verify a font loads and rasterizes\n");
    write_stdout("  bzfontd help                      show this message\n");
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

/// Read one line from stdin and split on whitespace into tokens.
fn read_args() -> Vec<String> {
    let mut buf = [0u8; 512];
    let bytes_read = raw::raw_read(0, buf.as_mut_ptr(), buf.len());
    if bytes_read <= 0 {
        return Vec::new();
    }

    let input = core::str::from_utf8(&buf[..bytes_read as usize]).unwrap_or("").trim();
    input
        .split_whitespace()
        .map(|token| {
            let mut owned = String::new();
            owned.push_str(token);
            owned
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn write_stdout(message: &str) {
    raw::raw_write(1, message.as_ptr(), message.len());
}

fn write_stderr(message: &str) {
    raw::raw_write(2, message.as_ptr(), message.len());
}

fn write_u32(value: u32) {
    if value == 0 {
        write_stdout("0");
        return;
    }
    let mut digits = [0u8; 10];
    let mut position = 10usize;
    let mut remaining = value;
    while remaining > 0 {
        position -= 1;
        digits[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    let text = core::str::from_utf8(&digits[position..]).unwrap_or("?");
    write_stdout(text);
}

// ---------------------------------------------------------------------------
// Panic / alloc-error handlers
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_write(2, b"bzfontd: panic\n".as_ptr(), 15);
    raw::raw_exit(1)
}

