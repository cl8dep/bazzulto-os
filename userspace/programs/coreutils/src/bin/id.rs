// POSIX — id
//
// Print real and effective user and group IDs.
// Output: uid=N(name) gid=N(name) [euid=N(name)] [egid=N(name)] [groups=N(name),...]

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::string::String;
use bazzulto_system::raw;
use bazzulto_system::environment::{Environment, SpecialFile};
use coreutils::write_stdout;

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);

    let uid  = raw::raw_getuid();
    let gid  = raw::raw_getgid();
    let euid = raw::raw_geteuid();
    let egid = raw::raw_getegid();

    let passwd = read_special_file(SpecialFile::Passwd);
    let group  = read_special_file(SpecialFile::Group);

    write_stdout("uid=");
    print_id(uid, passwd.as_deref(), true);

    write_stdout(" gid=");
    print_id(gid, group.as_deref(), false);

    if euid != uid {
        write_stdout(" euid=");
        print_id(euid, passwd.as_deref(), true);
    }
    if egid != gid {
        write_stdout(" egid=");
        print_id(egid, group.as_deref(), false);
    }

    let mut groups_buf = [0u32; 16];
    let ngroups = raw::raw_getgroups(16, groups_buf.as_mut_ptr());
    if ngroups > 0 {
        write_stdout(" groups=");
        for i in 0..ngroups as usize {
            if i > 0 { write_stdout(","); }
            print_id(groups_buf[i], group.as_deref(), false);
        }
    }

    write_stdout("\n");
    raw::raw_exit(0)
}

fn print_id(id: u32, db: Option<&str>, is_passwd: bool) {
    let mut buf = [0u8; 16];
    write_stdout(format_u32(id, &mut buf));
    if let Some(content) = db {
        if let Some(name) = resolve_name(id, content) {
            write_stdout("(");
            write_stdout(&name);
            write_stdout(")");
        }
    }
}

fn resolve_name(id: u32, content: &str) -> Option<String> {
    for line in content.lines() {
        let mut fields = line.splitn(4, ':');
        let name = fields.next()?;
        let _x = fields.next();
        let id_str = fields.next()?;
        if let Ok(file_id) = id_str.parse::<u32>() {
            if file_id == id { return Some(String::from(name)); }
        }
    }
    None
}

fn read_special_file(file: SpecialFile) -> Option<String> {
    let path = Environment::get_special_file(file);
    let mut path_buf = [0u8; 64];
    let plen = path.len().min(63);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    let fd = raw::raw_open(path_buf.as_ptr(), 0, 0);
    if fd < 0 { return None; }
    let mut buf = [0u8; 2048];
    let n = raw::raw_read(fd as i32, buf.as_mut_ptr(), buf.len());
    raw::raw_close(fd as i32);
    if n <= 0 { return None; }
    core::str::from_utf8(&buf[..n as usize]).ok().map(String::from)
}

fn format_u32(mut val: u32, buf: &mut [u8; 16]) -> &str {
    if val == 0 { return "0"; }
    let mut i = 15;
    while val > 0 { buf[i] = b'0' + (val % 10) as u8; val /= 10; i -= 1; }
    core::str::from_utf8(&buf[i + 1..]).unwrap_or("?")
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! { raw::raw_exit(1) }
