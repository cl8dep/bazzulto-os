// POSIX.1-2024 — ls
// https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/ls.html
//
// List directory contents.
//
// Supported options (POSIX base):
//   -a   Include entries beginning with '.'.
//   -A   Include entries beginning with '.' except '.' and '..'.
//   -d   Treat directories as plain entries; do not descend.
//   -F   Append type indicator: '/' dir, '*' exec, '@' symlink, '|' fifo.
//   -i   Precede each entry with its file serial number (inode).
//   -l   Long format: mode links owner group size date name.
//   -n   Like -l but print numeric UID/GID.
//   -p   Append '/' after directory names.
//   -q   Replace non-printable bytes in names with '?'.
//   -R   Recursively list subdirectories.
//   -r   Reverse sort order.
//   -S   Sort by size descending (primary), then name ascending.
//   -t   Sort by modification time descending (primary), then name ascending.
//   -1   One entry per line (default when not a terminal; we always use this).
//
// Output format constraints (POSIX §ls STDOUT):
//   Default: one entry per line.
//   With -l:  "%s %u %s %s %u %s %s\n"  for regular files
//             (mode  links  owner  group  size  date  name)
//   File-type indicator chars per -F: '/' dir, '*' executable,
//   '@' symlink, '|' FIFO, '=' socket.
//
// Implementation notes:
//   - Sorting locale: we sort byte-by-byte (POSIX locale collation).
//   - Timestamps: only mtime is available; date is omitted in -l output
//     (kernel fstat does not yet expose timestamps — marked TODO).
//   - Owner/group: not yet available from kernel (marked TODO).
//   - Block counts for "total N" header: omitted (kernel does not expose).
//   - -C/-m/-x (column modes) are not implemented; one-per-line is always used.
//   - Symlink resolution (-H/-L) is not implemented (no readlink syscall yet).
//
// Exit codes (POSIX §ls EXIT STATUS):
//   0  Successful completion.
//  >0  An error occurred.

#![no_std]
#![no_main]
extern crate alloc;
extern crate coreutils;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use bazzulto_system::raw;
use coreutils::{args, write_stdout, write_stderr};

// ---------------------------------------------------------------------------
// File type codes returned by sys_fstat [1] field.
// ---------------------------------------------------------------------------
const FTYPE_REGULAR:   u64 = 1;
const FTYPE_DIRECTORY: u64 = 2;
const FTYPE_CHARDEV:   u64 = 3;
const FTYPE_FIFO:      u64 = 4;
const FTYPE_SYMLINK:   u64 = 5;

// d_type values in linux_dirent64 (from kernel sys_getdents64 comment).
const DT_UNKNOWN: u8 = 0;
const DT_FIFO:    u8 = 1;
const DT_CHR:     u8 = 2;
const DT_DIR:     u8 = 4;
const DT_REG:     u8 = 8;
const DT_LNK:     u8 = 10;
const DT_SOCK:    u8 = 12;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

struct Options {
    show_all:       bool,   // -a: include '.' entries
    show_almost:    bool,   // -A: include '.' entries except '.' and '..'
    list_dir:       bool,   // -d: do not descend directories
    type_indicator: bool,   // -F: append type indicator
    inode:          bool,   // -i: print inode number
    long_format:    bool,   // -l or -n
    numeric_ids:    bool,   // -n: numeric UID/GID in long format
    slash_dirs:     bool,   // -p: append '/' to directories
    replace_ctrl:   bool,   // -q: replace non-printable with '?'
    recursive:      bool,   // -R: recurse into subdirectories
    reverse:        bool,   // -r: reverse sort
    sort_size:      bool,   // -S: sort by size
    sort_time:      bool,   // -t: sort by mtime
    one_per_line:   bool,   // -1: always one per line (our default)
}

impl Options {
    fn new() -> Self {
        Options {
            show_all:       false,
            show_almost:    false,
            list_dir:       false,
            type_indicator: false,
            inode:          false,
            long_format:    false,
            numeric_ids:    false,
            slash_dirs:     false,
            replace_ctrl:   false,
            recursive:      false,
            reverse:        false,
            sort_size:      false,
            sort_time:      false,
            one_per_line:   true,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry metadata
// ---------------------------------------------------------------------------

struct Entry {
    name:      String,
    inode_num: u64,
    file_size: u64,
    file_type: u64,   // FTYPE_* from fstat, or derived from d_type
    d_type:    u8,    // raw DT_* from getdents64
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve `path` to an absolute path using getcwd if needed.
fn resolve_path(path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let mut cwd_buf: Vec<u8> = alloc::vec![0u8; 512];
    let n = raw::raw_getcwd(cwd_buf.as_mut_ptr(), cwd_buf.len());
    if n <= 0 {
        return path.to_string();
    }
    let len = (n as usize).saturating_sub(1); // strip NUL
    let cwd = core::str::from_utf8(&cwd_buf[..len]).unwrap_or("/");
    if path == "." {
        return cwd.to_string();
    }
    alloc::format!("{}/{}", cwd.trim_end_matches('/'), path)
}

/// Call fstat for a path. Returns [size, file_type] or None.
fn fstat(path: &str) -> Option<[u64; 2]> {
    let mut buf = [0u64; 2];
    let ret = raw::raw_fstat(path.as_ptr(), path.len(), buf.as_mut_ptr());
    if ret < 0 { None } else { Some(buf) }
}

/// Replace non-printable bytes in `name` with '?' if -q is set.
fn maybe_sanitize(name: &str, replace_ctrl: bool) -> String {
    if !replace_ctrl {
        return name.to_string();
    }
    name.bytes()
        .map(|b| if b < 0x20 || b == 0x7F { b'?' } else { b })
        .map(|b| b as char)
        .collect()
}

/// POSIX file mode string: "drwxr-xr-x" etc.
/// Since the kernel does not yet expose permission bits, we use placeholder '-'
/// for all permission fields and derive the type character from file_type.
fn format_mode(file_type: u64) -> &'static str {
    match file_type {
        FTYPE_DIRECTORY => "d---------",
        FTYPE_SYMLINK   => "l---------",
        FTYPE_CHARDEV   => "c---------",
        FTYPE_FIFO      => "p---------",
        _               => "----------",
    }
}

/// File type indicator character per POSIX -F.
fn type_indicator(d_type: u8, file_type: u64) -> char {
    // Prefer d_type from getdents64 (no extra syscall).
    match d_type {
        DT_DIR  => '/',
        DT_LNK  => '@',
        DT_FIFO => '|',
        DT_SOCK => '=',
        DT_REG  => '\0', // executable check would need mode bits; omit for now
        _       => match file_type {
            FTYPE_DIRECTORY => '/',
            FTYPE_SYMLINK   => '@',
            FTYPE_FIFO      => '|',
            _               => '\0',
        },
    }
}

/// Write a decimal u64 to stdout.
fn write_u64(value: u64) {
    let mut buf = [0u8; 20];
    let mut pos = 20usize;
    let mut n = value;
    if n == 0 {
        write_stdout("0");
        return;
    }
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let slice = unsafe { core::str::from_utf8_unchecked(&buf[pos..]) };
    write_stdout(slice);
}

/// Right-pad `s` to `width` chars with spaces.
fn write_padded_right(s: &str, width: usize) {
    write_stdout(s);
    let len = s.len();
    if len < width {
        for _ in 0..(width - len) {
            write_stdout(" ");
        }
    }
}

/// Left-pad a u64 to `width` chars with spaces.
fn write_u64_right(value: u64, width: usize) {
    let mut buf = [0u8; 20];
    let mut pos = 20usize;
    let mut n = value;
    if n == 0 {
        buf[19] = b'0';
        pos = 19;
    } else {
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    let digits = 20 - pos;
    if digits < width {
        for _ in 0..(width - digits) {
            write_stdout(" ");
        }
    }
    let slice = unsafe { core::str::from_utf8_unchecked(&buf[pos..]) };
    write_stdout(slice);
}

// ---------------------------------------------------------------------------
// Read directory entries via getdents64
// ---------------------------------------------------------------------------

const DIRENT_HEADER: usize = 19; // d_ino(8) + d_off(8) + d_reclen(2) + d_type(1)

fn read_entries(dir_path: &str, opts: &Options) -> Vec<Entry> {
    let fd = raw::raw_open(dir_path.as_ptr(), dir_path.len());
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as i32;
    let mut entries: Vec<Entry> = Vec::new();
    let mut buf: Vec<u8> = alloc::vec![0u8; 4096];

    loop {
        let n = raw::raw_getdents64(fd, buf.as_mut_ptr(), buf.len());
        if n <= 0 { break; }
        let n = n as usize;
        let mut offset = 0usize;
        while offset < n {
            if offset + DIRENT_HEADER > n { break; }
            let inode_num = u64::from_ne_bytes(buf[offset..offset+8].try_into().unwrap_or([0;8]));
            let reclen = u16::from_ne_bytes([buf[offset+16], buf[offset+17]]) as usize;
            if reclen == 0 || offset + reclen > n { break; }
            let d_type = buf[offset + 18];
            let name_start = offset + DIRENT_HEADER;
            let name_end = buf[name_start..offset + reclen]
                .iter()
                .position(|&b| b == 0)
                .map(|p| name_start + p)
                .unwrap_or(offset + reclen);
            let name = core::str::from_utf8(&buf[name_start..name_end]).unwrap_or("");
            offset += reclen;

            if name.is_empty() { continue; }
            // Filter dot entries.
            if name == "." || name == ".." {
                if !opts.show_all { continue; }
                // -A excludes '.' and '..'; -a includes them.
                if opts.show_almost && !opts.show_all { continue; }
            } else if name.starts_with('.') && !opts.show_all && !opts.show_almost {
                continue;
            }

            // Derive file_type from d_type to avoid an fstat per entry.
            let file_type = match d_type {
                DT_REG  => FTYPE_REGULAR,
                DT_DIR  => FTYPE_DIRECTORY,
                DT_CHR  => FTYPE_CHARDEV,
                DT_FIFO => FTYPE_FIFO,
                DT_LNK  => FTYPE_SYMLINK,
                _       => 0u64,
            };

            entries.push(Entry {
                name: name.to_string(),
                inode_num,
                file_size: 0, // filled on demand for -l
                file_type,
                d_type,
            });
        }
    }

    raw::raw_close(fd);
    entries
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

fn sort_entries(entries: &mut Vec<Entry>, opts: &Options, dir_path: &str) {
    if opts.sort_size {
        // Ensure sizes are populated.
        for entry in entries.iter_mut() {
            if entry.file_size == 0 {
                let full = alloc::format!("{}/{}", dir_path.trim_end_matches('/'), entry.name);
                if let Some(stat) = fstat(&full) {
                    entry.file_size = stat[0];
                    if entry.file_type == 0 { entry.file_type = stat[1]; }
                }
            }
        }
        entries.sort_unstable_by(|a, b| {
            b.file_size.cmp(&a.file_size).then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
        });
    } else {
        // Default: alphabetical by POSIX locale (byte order).
        // sort_unstable_by avoids the auxiliary stack buffer that sort_by uses,
        // which would overflow the user-space stack on large directories.
        entries.sort_unstable_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    }
    if opts.reverse {
        entries.reverse();
    }
}

// ---------------------------------------------------------------------------
// Output: default (one per line)
// ---------------------------------------------------------------------------

fn print_entry_default(entry: &Entry, opts: &Options, dir_path: &str, full_path: &str) {
    let display = maybe_sanitize(&entry.name, opts.replace_ctrl);

    if opts.inode {
        write_u64(entry.inode_num);
        write_stdout(" ");
    }

    write_stdout(&display);

    // Append type indicator (-F) or directory slash (-p).
    if opts.type_indicator {
        let indicator = type_indicator(entry.d_type, entry.file_type);
        if indicator != '\0' {
            let mut tmp = [0u8; 4];
            let s = indicator.encode_utf8(&mut tmp);
            write_stdout(s);
        }
    } else if opts.slash_dirs {
        if entry.file_type == FTYPE_DIRECTORY || entry.d_type == DT_DIR {
            write_stdout("/");
        }
    }

    write_stdout("\n");
}

// ---------------------------------------------------------------------------
// Output: long format (-l / -n)
// ---------------------------------------------------------------------------

fn print_entry_long(entry: &Entry, opts: &Options, full_path: &str) {
    // Get full metadata.
    let (size, file_type) = match fstat(full_path) {
        Some(stat) => (stat[0], stat[1]),
        None       => (entry.file_size, entry.file_type),
    };

    let mode = format_mode(file_type);
    let display = maybe_sanitize(&entry.name, opts.replace_ctrl);

    // POSIX format: "%s %u %s %s %u %s %s\n"
    //   mode  links  owner  group  size  date  name
    //
    // Kernel does not yet expose links, owner, group, or timestamps.
    // We emit placeholders to keep the column layout readable.
    //  mode(10) + space + nlinks(1+) + space + owner(1+) + space + group(1+)
    //  + space + size(right-aligned) + space + date(12) + space + name

    if opts.inode {
        write_u64(entry.inode_num);
        write_stdout(" ");
    }

    write_stdout(mode);
    write_stdout(" ");
    write_stdout("1"); // hard link count — TODO: expose via fstat
    write_stdout(" ");
    if opts.numeric_ids {
        write_stdout("0"); // UID — TODO
        write_stdout(" ");
        write_stdout("0"); // GID — TODO
    } else {
        write_stdout("root"); // owner — TODO
        write_stdout(" ");
        write_stdout("root"); // group — TODO
    }
    write_stdout(" ");
    write_u64_right(size, 8);
    write_stdout(" ");
    // Date: TODO — kernel fstat does not yet expose timestamps.
    // POSIX requires "Mmm dd HH:MM" (recent) or "Mmm dd  YYYY" (old).
    // Emit a placeholder that is the right width.
    write_stdout("Jan  1  1970");
    write_stdout(" ");
    write_stdout(&display);

    // Append type indicator if -F.
    if opts.type_indicator {
        let indicator = type_indicator(entry.d_type, file_type);
        if indicator != '\0' {
            let mut tmp = [0u8; 4];
            let s = indicator.encode_utf8(&mut tmp);
            write_stdout(s);
        }
    }

    write_stdout("\n");
}

// ---------------------------------------------------------------------------
// List one directory
// ---------------------------------------------------------------------------

fn list_directory(dir_path: &str, opts: &Options, print_header: bool) -> bool {
    let resolved = resolve_path(dir_path);

    // Verify it is a directory (or handle -d).
    if opts.list_dir {
        // -d: treat the operand as a plain file, do not descend.
        let stat = fstat(&resolved);
        let file_type = stat.map(|s| s[1]).unwrap_or(0);
        let size      = stat.map(|s| s[0]).unwrap_or(0);
        let entry = Entry {
            name:      dir_path.to_string(),
            inode_num: 0,
            file_size: size,
            file_type,
            d_type:    if file_type == FTYPE_DIRECTORY { DT_DIR } else { DT_REG },
        };
        let full = resolved.clone();
        if opts.long_format {
            print_entry_long(&entry, opts, &full);
        } else {
            print_entry_default(&entry, opts, &resolved, &full);
        }
        return true;
    }

    let mut entries = read_entries(&resolved, opts);
    if entries.is_empty() {
        // Distinguish "not found" from "empty directory".
        let fd = raw::raw_open(resolved.as_ptr(), resolved.len());
        if fd < 0 {
            write_stderr("ls: cannot access '");
            write_stderr(dir_path);
            write_stderr("': No such file or directory\n");
            return false;
        }
        raw::raw_close(fd as i32);
        // Empty directory — print header if multi-dir listing, then return.
        if print_header {
            write_stdout("\n");
            write_stdout(dir_path);
            write_stdout(":\n");
        }
        return true;
    }

    sort_entries(&mut entries, opts, &resolved);

    if print_header {
        write_stdout("\n");
        write_stdout(dir_path);
        write_stdout(":\n");
    }

    // Collect subdirectories for -R before printing.
    let mut subdirs: Vec<String> = Vec::new();

    for entry in &entries {
        let full = alloc::format!("{}/{}", resolved.trim_end_matches('/'), entry.name);
        if opts.long_format {
            print_entry_long(entry, opts, &full);
        } else {
            print_entry_default(entry, opts, &resolved, &full);
        }
        if opts.recursive
            && (entry.d_type == DT_DIR || entry.file_type == FTYPE_DIRECTORY)
            && entry.name != "."
            && entry.name != ".."
        {
            subdirs.push(full);
        }
    }

    // -R: recurse.
    for subdir in &subdirs {
        list_directory(subdir, opts, true);
    }

    true
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
    bazzulto_system::init_with_args_envp(argc, argv, envp);
    let arguments = args();

    let mut opts = Options::new();
    let mut operands: Vec<&str> = Vec::new();
    let mut end_of_options = false;

    for arg in arguments[1..].iter() {
        let s = arg.as_str();
        if end_of_options || !s.starts_with('-') || s == "-" {
            operands.push(s);
            continue;
        }
        if s == "--" {
            end_of_options = true;
            continue;
        }
        for ch in s[1..].chars() {
            match ch {
                'a' => { opts.show_all    = true; opts.show_almost = false; }
                'A' => { if !opts.show_all { opts.show_almost = true; } }
                'd' => opts.list_dir       = true,
                'F' => opts.type_indicator = true,
                'i' => opts.inode          = true,
                'l' => opts.long_format    = true,
                'n' => { opts.long_format = true; opts.numeric_ids = true; }
                'p' => opts.slash_dirs     = true,
                'q' => opts.replace_ctrl   = true,
                'R' => opts.recursive      = true,
                'r' => opts.reverse        = true,
                'S' => { opts.sort_size = true; opts.sort_time = false; }
                't' => { opts.sort_time = true; opts.sort_size = false; }
                '1' => opts.one_per_line   = true,
                // Options whose output mode conflicts with long format
                // (-C, -m, -x): last option wins (POSIX §ls OPTIONS). We don't
                // implement column modes so just accept without effect.
                'C' | 'm' | 'x' => {}
                // XSI options that imply long format.
                'g' => opts.long_format = true,
                'o' => opts.long_format = true,
                // Options not yet implemented (no syscall support).
                'H' | 'L' | 'c' | 'u' | 'k' | 's' => {}
                other => {
                    write_stderr("ls: invalid option -- '");
                    let mut tmp = [0u8; 4];
                    let s = other.encode_utf8(&mut tmp);
                    write_stderr(s);
                    write_stderr("'\n");
                    write_stderr("usage: ls [-AaFdFilnpqRrSt1] [file...]\n");
                    raw::raw_exit(1);
                }
            }
        }
    }

    // Default operand: current directory.
    if operands.is_empty() {
        operands.push(".");
    }

    // Partition operands into non-directories (printed first) and directories.
    // For a single operand that is a directory, suppress the directory header.
    let only_one = operands.len() == 1;
    let mut any_error = false;

    // Separate non-directory operands (printed first per POSIX).
    let mut non_dir_operands: Vec<&str> = Vec::new();
    let mut dir_operands:     Vec<&str> = Vec::new();

    for &op in &operands {
        let resolved = resolve_path(op);
        if let Some(stat) = fstat(&resolved) {
            if stat[1] == FTYPE_DIRECTORY && !opts.list_dir {
                dir_operands.push(op);
            } else {
                non_dir_operands.push(op);
            }
        } else {
            // Doesn't exist — will be reported by list_directory or printed as error.
            write_stderr("ls: cannot access '");
            write_stderr(op);
            write_stderr("': No such file or directory\n");
            any_error = true;
        }
    }

    // Print non-directory operands first.
    for &op in &non_dir_operands {
        let resolved = resolve_path(op);
        let stat = fstat(&resolved);
        let (size, file_type) = stat.map(|s| (s[0], s[1])).unwrap_or((0, 0));
        let entry = Entry {
            name:      op.to_string(),
            inode_num: 0,
            file_size: size,
            file_type,
            d_type:    if file_type == FTYPE_DIRECTORY { DT_DIR } else { DT_REG },
        };
        if opts.long_format {
            print_entry_long(&entry, &opts, &resolved);
        } else {
            print_entry_default(&entry, &opts, &resolved, &resolved);
        }
    }

    // Print directory operands.
    let multi = dir_operands.len() > 1 || !non_dir_operands.is_empty();
    let mut first_dir = true;
    for &op in &dir_operands {
        let print_header = multi || opts.recursive;
        if print_header {
            if !first_dir || !non_dir_operands.is_empty() {
                write_stdout("\n");
            }
            write_stdout(op);
            write_stdout(":\n");
        }
        first_dir = false;
        if !list_directory_no_header(op, &opts) {
            any_error = true;
        }
    }

    raw::raw_exit(if any_error { 1 } else { 0 });
}

/// Like `list_directory` but without printing the leading header line
/// (the caller already printed it for multi-dir listings).
fn list_directory_no_header(dir_path: &str, opts: &Options) -> bool {
    let resolved = resolve_path(dir_path);

    if opts.list_dir {
        let stat = fstat(&resolved);
        let file_type = stat.map(|s| s[1]).unwrap_or(0);
        let size      = stat.map(|s| s[0]).unwrap_or(0);
        let entry = Entry {
            name:      dir_path.to_string(),
            inode_num: 0,
            file_size: size,
            file_type,
            d_type:    if file_type == FTYPE_DIRECTORY { DT_DIR } else { DT_REG },
        };
        if opts.long_format {
            print_entry_long(&entry, opts, &resolved);
        } else {
            print_entry_default(&entry, opts, &resolved, &resolved);
        }
        return true;
    }

    let mut entries = read_entries(&resolved, opts);
    if entries.is_empty() {
        let fd = raw::raw_open(resolved.as_ptr(), resolved.len());
        if fd < 0 {
            write_stderr("ls: cannot access '");
            write_stderr(dir_path);
            write_stderr("': No such file or directory\n");
            return false;
        }
        raw::raw_close(fd as i32);
        return true;
    }

    sort_entries(&mut entries, opts, &resolved);

    let mut subdirs: Vec<String> = Vec::new();

    for entry in &entries {
        let full = alloc::format!("{}/{}", resolved.trim_end_matches('/'), entry.name);
        if opts.long_format {
            print_entry_long(entry, opts, &full);
        } else {
            print_entry_default(entry, opts, &resolved, &full);
        }
        if opts.recursive
            && (entry.d_type == DT_DIR || entry.file_type == FTYPE_DIRECTORY)
            && entry.name != "."
            && entry.name != ".."
        {
            subdirs.push(full);
        }
    }

    for subdir in &subdirs {
        write_stdout("\n");
        write_stdout(subdir);
        write_stdout(":\n");
        list_directory_no_header(subdir, opts);
    }

    true
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    raw::raw_exit(1)
}
