// POSIX — whoami
//
// Print the user name associated with the current effective user ID.
// Resolves from the system passwd file via Environment::get_special_file().

#![no_std]
#![no_main]
extern crate coreutils;

use bazzulto_system::raw;
use bazzulto_system::environment::{Environment, SpecialFile};

#[no_mangle]
pub extern "C" fn _start(_argc: usize, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    let euid = raw::raw_geteuid();

    // Open and read passwd file.
    let path = Environment::get_special_file(SpecialFile::Passwd);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);

    if fd >= 0 {
        let mut buf = [0u8; 1024];
        let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
        raw::raw_close(fd as i32);

        if n > 0 {
            // Parse passwd lines to find our euid.
            if let Some(name) = find_username(&buf[..n as usize], euid) {
                raw::raw_write(1, name.as_ptr(), name.len());
                raw::raw_write(1, b"\n".as_ptr(), 1);
                raw::raw_exit(0);
            }
        }
    }

    // Fallback: print numeric euid.
    print_u32(euid);
    raw::raw_write(1, b"\n".as_ptr(), 1);
    raw::raw_exit(0)
}

/// Find the username for `uid` in passwd file content. Returns a byte slice.
fn find_username<'a>(content: &'a [u8], uid: u32) -> Option<&'a [u8]> {
    let mut start = 0;
    while start < content.len() {
        // Find end of line.
        let end = content[start..].iter().position(|&b| b == b'\n')
            .map(|p| start + p)
            .unwrap_or(content.len());
        let line = &content[start..end];
        start = end + 1;

        // Parse: name:x:uid:...
        let mut colons = [0usize; 3];
        let mut colon_count = 0;
        for (i, &b) in line.iter().enumerate() {
            if b == b':' {
                if colon_count < 3 { colons[colon_count] = i; }
                colon_count += 1;
                if colon_count >= 3 { break; }
            }
        }
        if colon_count < 3 { continue; }

        // Parse uid field (between colon[1]+1 and colon[2]).
        let uid_bytes = &line[colons[1]+1..colons[2]];
        let mut file_uid: u32 = 0;
        let mut valid = true;
        for &b in uid_bytes {
            if b >= b'0' && b <= b'9' {
                file_uid = file_uid * 10 + (b - b'0') as u32;
            } else {
                valid = false;
                break;
            }
        }
        if valid && file_uid == uid {
            return Some(&line[..colons[0]]);
        }
    }
    None
}

fn print_u32(mut val: u32) {
    if val == 0 {
        raw::raw_write(1, b"0".as_ptr(), 1);
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 9;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        if i == 0 { break; }
        i -= 1;
    }
    raw::raw_write(1, buf[i+1..].as_ptr(), 10 - (i + 1));
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
