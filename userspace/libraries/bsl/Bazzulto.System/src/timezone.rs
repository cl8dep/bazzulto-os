//! Timezone support — IANA TZif binary files + POSIX TZ strings.
//!
//! This module provides the same timezone resolution as Linux and macOS:
//!
//! 1. Read the `$TZ` environment variable.
//!    - If it is an IANA name like `America/New_York`, load the TZif binary
//!      from `/system/share/zoneinfo/<name>`.
//!    - If it looks like a POSIX rule string (`EST5EDT,...`), parse it directly.
//!    - If it is `UTC`, `UTC0`, or empty, use UTC.
//!    - If the variable is unset, read `/etc/localtime` as a TZif binary.
//! 2. Fall back to UTC if nothing is found.
//!
//! The TZif format is defined by RFC 8536.  We support v1 and v2/v3 files.
//!
//! # Public surface
//!
//! ```no_run
//! use bazzulto_system::timezone::{resolve_timezone, Timezone};
//!
//! let tz   = resolve_timezone();          // reads $TZ / /etc/localtime
//! let offs = tz.utc_offset_at(unix_ts);  // seconds east of UTC
//! let abbr = tz.abbreviation_at(unix_ts);
//! let dst  = tz.is_dst_at(unix_ts);
//! ```
//!
//! # Caching
//!
//! `resolve_timezone()` caches the result day-granularly (the cached value is
//! refreshed at most once per calendar day).  This is sufficient for all
//! `date`-command and log-timestamp use cases.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::raw;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A resolved timezone.
///
/// This is either a table-driven IANA zone (loaded from a TZif binary) or a
/// POSIX rule string.  UTC is represented as a fixed-offset zone with offset 0.
#[derive(Clone)]
pub struct Timezone {
    inner: TimezoneInner,
}

#[derive(Clone)]
enum TimezoneInner {
    /// Fixed offset from UTC (seconds east).  Covers UTC, UTC+N, etc.
    Fixed { offset_seconds: i32, abbreviation: String },
    /// Table-driven zone from a parsed TZif binary.
    Table(TzTable),
    /// POSIX TZ rule string (no TZif available).
    Posix(PosixRule),
}

/// A table of transition times extracted from a TZif binary.
#[derive(Clone)]
struct TzTable {
    /// Transition times as Unix timestamps (sorted, ascending).
    transition_times: Vec<i64>,
    /// Index into `type_info` for each transition.
    transition_types: Vec<u8>,
    /// UTC offset (seconds east), DST flag, abbreviation index per type.
    type_info: Vec<TtInfo>,
    /// Abbreviation strings (NUL-separated flat buffer).
    abbr_buf: Vec<u8>,
    /// POSIX rule string from the TZif v2/v3 footer (may be empty).
    footer_posix: String,
}

/// A single ttinfo record from a TZif file.
#[derive(Clone, Copy)]
struct TtInfo {
    utoff:  i32,   // seconds east of UTC
    dst:    bool,
    abbr_i: u8,    // byte index into abbr_buf
}

/// A POSIX TZ rule string (e.g. `EST5EDT,M3.2.0,M11.1.0`).
#[derive(Clone)]
pub struct PosixRule {
    std_name:   String,
    std_offset: i32,           // seconds east of UTC (note: POSIX sign is inverted; stored corrected)
    dst_name:   String,
    dst_offset: i32,
    dst_start:  Option<PosixTransition>,
    dst_end:    Option<PosixTransition>,
}

/// A DST transition rule in a POSIX TZ string.
#[derive(Clone, Copy)]
struct PosixTransition {
    kind:   TransitionKind,
    time:   i32, // seconds from midnight (default 7200 = 02:00)
}

#[derive(Clone, Copy)]
enum TransitionKind {
    /// `Jn` — Julian day 1–365 (no leap day).
    Julian(u16),
    /// `n`  — zero-based Julian day 0–365.
    Day(u16),
    /// `Mm.w.d` — month, week (1=first, 5=last), day-of-week (0=Sun).
    MonthWeekDay { month: u8, week: u8, day: u8 },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl Timezone {
    /// UTC timezone (zero offset, no DST).
    pub fn utc() -> Timezone {
        Timezone {
            inner: TimezoneInner::Fixed {
                offset_seconds: 0,
                abbreviation:   String::from("UTC"),
            },
        }
    }

    /// UTC offset at a given Unix timestamp (seconds east of UTC).
    pub fn utc_offset_at(&self, unix_seconds: i64) -> i32 {
        match &self.inner {
            TimezoneInner::Fixed { offset_seconds, .. } => *offset_seconds,
            TimezoneInner::Table(t)  => t.utc_offset_at(unix_seconds),
            TimezoneInner::Posix(p)  => p.utc_offset_at(unix_seconds),
        }
    }

    /// Whether DST is active at the given Unix timestamp.
    pub fn is_dst_at(&self, unix_seconds: i64) -> bool {
        match &self.inner {
            TimezoneInner::Fixed { .. } => false,
            TimezoneInner::Table(t)     => t.is_dst_at(unix_seconds),
            TimezoneInner::Posix(p)     => p.is_dst_at(unix_seconds),
        }
    }

    /// Abbreviated timezone name at the given Unix timestamp (e.g. `"EST"`, `"EDT"`).
    pub fn abbreviation_at(&self, unix_seconds: i64) -> &str {
        match &self.inner {
            TimezoneInner::Fixed { abbreviation, .. } => abbreviation.as_str(),
            TimezoneInner::Table(t)  => t.abbreviation_at(unix_seconds),
            TimezoneInner::Posix(p)  => p.abbreviation_at(unix_seconds),
        }
    }

    /// Format the UTC offset as `+HHMM` or `-HHMM` (POSIX `%z`).
    pub fn format_offset(&self, unix_seconds: i64) -> String {
        let off = self.utc_offset_at(unix_seconds);
        let (sign, abs) = if off >= 0 { ('+', off) } else { ('-', -off) };
        let hh = abs / 3600;
        let mm = (abs % 3600) / 60;
        let mut s = String::new();
        s.push(sign);
        push_u8_2(&mut s, hh as u8);
        push_u8_2(&mut s, mm as u8);
        s
    }
}

// ---------------------------------------------------------------------------
// Global cache — refreshed at most once per calendar day
// ---------------------------------------------------------------------------

/// Cached timezone, stored as a raw pointer to a heap-allocated `Timezone`.
///
/// `0` means "not cached yet".  We only set this once (or when the day changes).
static CACHED_TZ_PTR:  AtomicU64 = AtomicU64::new(0);
/// Unix timestamp of the start of the cached day (midnight UTC, seconds).
static CACHED_DAY_UTC: AtomicU64 = AtomicU64::new(0);

/// Resolve the current process timezone from the environment, with caching.
///
/// Resolution order:
/// 1. `$TZ` environment variable (IANA name, POSIX rule, or `UTC`).
/// 2. `/etc/localtime` TZif binary.
/// 3. UTC fallback.
pub fn resolve_timezone() -> Timezone {
    let now_secs = {
        let mut buf = [0u64; 2];
        raw::raw_clock_gettime(0 /* CLOCK_REALTIME */, buf.as_mut_ptr());
        buf[0]
    };
    let today_start = (now_secs / 86400) * 86400;
    let cached_day  = CACHED_DAY_UTC.load(Ordering::Relaxed);

    if cached_day == today_start {
        let ptr = CACHED_TZ_PTR.load(Ordering::Relaxed);
        if ptr != 0 {
            // Safety: we wrote a valid `Box<Timezone>` into this pointer and
            // never free it while the process is alive.
            let tz_ref: &Timezone = unsafe { &*(ptr as *const Timezone) };
            return tz_ref.clone();
        }
    }

    let tz = resolve_timezone_uncached();

    // Leak a Box<Timezone> so the pointer stays valid.
    let boxed = alloc::boxed::Box::new(tz.clone());
    let ptr = alloc::boxed::Box::into_raw(boxed) as u64;

    // Free previous allocation if day rolled over.
    let old_ptr = CACHED_TZ_PTR.swap(ptr, Ordering::Relaxed);
    if old_ptr != 0 {
        // Safety: we allocated this pointer ourselves above.
        drop(unsafe { alloc::boxed::Box::from_raw(old_ptr as *mut Timezone) });
    }
    CACHED_DAY_UTC.store(today_start, Ordering::Relaxed);

    tz
}

/// Resolve without the cache.
fn resolve_timezone_uncached() -> Timezone {
    // 1. Try $TZ.
    if let Some(tz_str) = get_env_tz() {
        if tz_str.is_empty() || tz_str == "UTC" || tz_str == "UTC0" {
            return Timezone::utc();
        }
        // POSIX rule strings contain '/' or start with ':', or have digits
        // early on.  IANA names never start with ':' but may contain '/'.
        // Heuristic: if it contains no '/' and has a digit in position [3..6],
        // it is a POSIX string.  If it starts with ':', strip the colon first.
        let tz_str = if tz_str.starts_with(':') { &tz_str[1..] } else { tz_str.as_str() };
        // IANA name: contains '/' or matches the zoneinfo directory.
        if is_likely_iana_name(tz_str) {
            if let Some(tz) = load_tzif_by_name(tz_str) {
                return tz;
            }
        }
        // Try as POSIX rule.
        if let Some(tz) = parse_posix_tz_str(tz_str) {
            return Timezone { inner: TimezoneInner::Posix(tz) };
        }
    }

    // 2. Try /etc/localtime.
    if let Some(tz) = load_tzif_file("/etc/localtime") {
        return tz;
    }

    // 3. UTC fallback.
    Timezone::utc()
}

/// Read the `$TZ` environment variable using the process envp.
fn get_env_tz() -> Option<String> {
    let envp = crate::envp_raw();
    if envp.is_null() {
        return None;
    }
    let mut index = 0usize;
    loop {
        let ptr = unsafe { *envp.add(index) };
        if ptr.is_null() {
            break;
        }
        // Read NUL-terminated string.
        let mut len = 0usize;
        loop {
            if unsafe { *ptr.add(len) } == 0 { break; }
            len += 1;
            if len > 4096 { break; }
        }
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        if let Ok(s) = core::str::from_utf8(bytes) {
            if let Some(val) = s.strip_prefix("TZ=") {
                return Some(String::from(val));
            }
        }
        index += 1;
    }
    None
}

/// Return true if `s` looks like an IANA timezone name (`Region/City` form
/// or a single-component name known to be in the zoneinfo database).
fn is_likely_iana_name(s: &str) -> bool {
    // Contains a '/' → almost certainly an IANA name.
    if s.contains('/') { return true; }
    // Single-component IANA names: UTC, GMT, EST, CST, ... but those are
    // ambiguous with POSIX; only treat them as IANA if they contain no digits.
    s.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-' || b == b'+')
        && !s.is_empty()
}

/// Load a TZif file given an IANA name like `"America/New_York"`.
///
/// Looks up `/system/share/zoneinfo/<name>`.
fn load_tzif_by_name(name: &str) -> Option<Timezone> {
    let mut path = String::from("/system/share/zoneinfo/");
    path.push_str(name);
    load_tzif_file(path.as_str())
}

// ---------------------------------------------------------------------------
// TZif binary parser (RFC 8536)
// ---------------------------------------------------------------------------

/// Maximum size we are willing to read for a TZif file.
const MAX_TZIF_SIZE: usize = 128 * 1024;

/// Read a TZif file and parse it into a `Timezone`.
pub fn load_tzif_file(path: &str) -> Option<Timezone> {
    let fd = raw::raw_open(path.as_ptr(), path.len());
    if fd < 0 { return None; }
    let fd = fd as i32;

    // Read up to MAX_TZIF_SIZE bytes.
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    buf.resize(MAX_TZIF_SIZE, 0u8);
    let mut total = 0usize;
    loop {
        let n = raw::raw_read(fd, buf[total..].as_mut_ptr(), MAX_TZIF_SIZE - total);
        if n <= 0 { break; }
        total += n as usize;
        if total >= MAX_TZIF_SIZE { break; }
    }
    raw::raw_close(fd);
    buf.truncate(total);

    parse_tzif(&buf)
}

/// Parse a TZif binary blob into a `Timezone`.
fn parse_tzif(data: &[u8]) -> Option<Timezone> {
    // Minimum header size: 44 bytes.
    if data.len() < 44 { return None; }

    // Magic: "TZif"
    if &data[0..4] != b"TZif" { return None; }

    // Version byte at offset 4: '\0', '2', or '3'.
    let version = data[4];

    // Parse v1 header first (always present).
    let v1_table = parse_tzif_block(data, 44, version == b'2' || version == b'3')?;

    // For v2/v3 files, skip the v1 data block and parse the v2 block.
    if version == b'2' || version == b'3' {
        let v1_size = tzif_block_size(&data[0..], 44)?;
        let v2_start = 44 + v1_size;
        if v2_start + 44 <= data.len() {
            if let Some(mut v2_table) = parse_tzif_block(&data[v2_start..], 44, false) {
                // Extract POSIX footer: after the v2 data block, terminated by '\n'.
                let v2_size = tzif_block_size(&data[v2_start..], 44)?;
                let footer_start = v2_start + 44 + v2_size;
                if footer_start < data.len() && data[footer_start] == b'\n' {
                    let footer_data = &data[footer_start + 1..];
                    if let Some(end) = footer_data.iter().position(|&b| b == b'\n') {
                        if let Ok(s) = core::str::from_utf8(&footer_data[..end]) {
                            v2_table.footer_posix = String::from(s);
                        }
                    }
                }
                return Some(Timezone { inner: TimezoneInner::Table(v2_table) });
            }
        }
    }

    Some(Timezone { inner: TimezoneInner::Table(v1_table) })
}

/// Read the 6 count fields from a TZif header starting at `header_offset`.
///
/// Returns `(ttisgmtcnt, ttisstdcnt, leapcnt, timecnt, typecnt, charcnt)`.
fn read_tzif_counts(data: &[u8], header_offset: usize) -> Option<(usize,usize,usize,usize,usize,usize)> {
    if data.len() < header_offset + 44 { return None; }
    let h = &data[header_offset..header_offset + 44];
    let ttisgmtcnt = u32_be(&h[20..24]) as usize;
    let ttisstdcnt = u32_be(&h[24..28]) as usize;
    let leapcnt    = u32_be(&h[28..32]) as usize;
    let timecnt    = u32_be(&h[32..36]) as usize;
    let typecnt    = u32_be(&h[36..40]) as usize;
    let charcnt    = u32_be(&h[40..44]) as usize;
    Some((ttisgmtcnt, ttisstdcnt, leapcnt, timecnt, typecnt, charcnt))
}

/// Compute the byte size of a TZif data block (excluding the 44-byte header).
///
/// `data` must start at the beginning of the TZif data (after the 44-byte
/// header is not included here — `data` is the full slice starting at magic).
/// `header_offset` is always 0 or 44 (for the second block in v2 files).
fn tzif_block_size(data: &[u8], header_offset: usize) -> Option<usize> {
    let (ttisgmtcnt, ttisstdcnt, leapcnt, timecnt, typecnt, charcnt) =
        read_tzif_counts(data, header_offset)?;
    // v1 sizes (4-byte transition times, 4-byte leap pairs).
    let size = timecnt * 4       // transition times (i32 or i64 in v2)
        + timecnt * 1            // transition types (u8)
        + typecnt * 6            // ttinfo records (i32 utoff, u8 dst, u8 abbr_i)
        + charcnt                // abbreviation strings
        + leapcnt * 8            // leap second pairs (i32 time + i32 correction)
        + ttisstdcnt             // UT/wall flags
        + ttisgmtcnt;            // UTC/local flags
    Some(size)
}

/// Parse a TZif data block starting at `data[header_offset..]`.
///
/// `large_transitions`: if true, transition times are 8-byte (v2/v3); otherwise 4-byte.
fn parse_tzif_block(data: &[u8], header_offset: usize, large_transitions: bool) -> Option<TzTable> {
    let (_, _, leapcnt, timecnt, typecnt, charcnt) =
        read_tzif_counts(data, header_offset)?;

    let base = header_offset + 44; // start of data after header
    let time_size = if large_transitions { 8 } else { 4 };

    // Section offsets within the data block.
    let trans_times_offset  = base;
    let trans_types_offset  = trans_times_offset + timecnt * time_size;
    let ttinfo_offset       = trans_types_offset + timecnt;
    let abbr_offset         = ttinfo_offset      + typecnt * 6;
    let end_of_data         = abbr_offset        + charcnt
        + leapcnt * (if large_transitions { 12 } else { 8 });

    if end_of_data > data.len() { return None; }

    // Parse transition times.
    let mut transition_times: Vec<i64> = Vec::with_capacity(timecnt);
    for i in 0..timecnt {
        let off = trans_times_offset + i * time_size;
        let t = if large_transitions {
            i64_be(&data[off..off+8])
        } else {
            i32_be(&data[off..off+4]) as i64
        };
        transition_times.push(t);
    }

    // Parse transition types.
    let transition_types: Vec<u8> = data[trans_types_offset..trans_types_offset+timecnt].to_vec();

    // Parse ttinfo records.
    let mut type_info: Vec<TtInfo> = Vec::with_capacity(typecnt);
    for i in 0..typecnt {
        let off = ttinfo_offset + i * 6;
        if off + 6 > data.len() { return None; }
        let utoff  = i32_be(&data[off..off+4]);
        let dst    = data[off+4] != 0;
        let abbr_i = data[off+5];
        type_info.push(TtInfo { utoff, dst, abbr_i });
    }

    // Copy abbreviation buffer.
    let abbr_buf: Vec<u8> = data[abbr_offset..abbr_offset+charcnt].to_vec();

    Some(TzTable {
        transition_times,
        transition_types,
        type_info,
        abbr_buf,
        footer_posix: String::new(),
    })
}

// ---------------------------------------------------------------------------
// TzTable lookup
// ---------------------------------------------------------------------------

impl TzTable {
    /// Find the ttinfo applicable at `unix_seconds` using binary search.
    fn find_ttinfo(&self, unix_seconds: i64) -> Option<&TtInfo> {
        if self.transition_times.is_empty() {
            return self.type_info.first();
        }
        // Binary search: find the last transition time <= unix_seconds.
        let pos = match self.transition_times.binary_search(&unix_seconds) {
            Ok(i) => i,
            Err(0) => {
                // Before the first transition — use type 0 (or first non-DST type).
                return self.type_info.first();
            }
            Err(i) => i - 1,
        };
        let type_index = *self.transition_types.get(pos)? as usize;
        self.type_info.get(type_index)
    }

    fn utc_offset_at(&self, unix_seconds: i64) -> i32 {
        self.find_ttinfo(unix_seconds).map(|t| t.utoff).unwrap_or(0)
    }

    fn is_dst_at(&self, unix_seconds: i64) -> bool {
        self.find_ttinfo(unix_seconds).map(|t| t.dst).unwrap_or(false)
    }

    fn abbreviation_at(&self, unix_seconds: i64) -> &str {
        let abbr_i = self.find_ttinfo(unix_seconds)
            .map(|t| t.abbr_i as usize)
            .unwrap_or(0);
        // Abbreviation is a NUL-terminated string starting at abbr_i in abbr_buf.
        if abbr_i >= self.abbr_buf.len() { return "UTC"; }
        let slice = &self.abbr_buf[abbr_i..];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        core::str::from_utf8(&slice[..end]).unwrap_or("UTC")
    }
}

// ---------------------------------------------------------------------------
// POSIX TZ string parser
// ---------------------------------------------------------------------------
//
// Grammar (from POSIX.1-2024):
//
//   std offset [dst [offset] [, rule , rule]]
//
//   std / dst   = name (alphabetic or '<...>')
//   offset      = [+-] hh [:mm [:ss]]
//   rule        = J n | n | M m.w.d
//   time        = hh [:mm [:ss]]   (seconds east; default 02:00 for dst transitions)
//
// NOTE: POSIX offsets are west-of-UTC (inverted from the common convention).
// We store them as seconds EAST internally.

/// Parse a POSIX TZ rule string into a `PosixRule`.
pub fn parse_posix_tz_str(s: &str) -> Option<PosixRule> {
    let bytes = s.as_bytes();
    let mut pos = 0usize;

    // Parse std name.
    let (std_name, consumed) = parse_tz_name(bytes, pos)?;
    pos += consumed;

    // Parse std offset (mandatory).
    let (std_offset_posix, consumed) = parse_tz_offset(bytes, pos)?;
    let std_offset = -std_offset_posix; // convert POSIX west-of-UTC → east
    pos += consumed;

    // Optional DST section.
    if pos >= bytes.len() {
        return Some(PosixRule {
            std_name,
            std_offset,
            dst_name: String::new(),
            dst_offset: std_offset + 3600, // conventional 1-hour DST
            dst_start: None,
            dst_end:   None,
        });
    }

    // DST name.
    let (dst_name, consumed) = parse_tz_name(bytes, pos)?;
    pos += consumed;

    // Optional DST offset (default: std_offset + 1 hour).
    let (dst_offset, consumed_off) = if pos < bytes.len()
        && (bytes[pos] == b'+' || bytes[pos] == b'-'
            || (bytes[pos] >= b'0' && bytes[pos] <= b'9')) {
        let (v, c) = parse_tz_offset(bytes, pos)?;
        (-v, c) // convert
    } else {
        (std_offset + 3600, 0)
    };
    pos += consumed_off;

    // Optional DST transition rules.
    let (dst_start, dst_end) = if pos < bytes.len() && bytes[pos] == b',' {
        pos += 1;
        let (start, consumed) = parse_tz_rule(bytes, pos)?;
        pos += consumed;
        if pos >= bytes.len() || bytes[pos] != b',' { return None; }
        pos += 1;
        let (end, consumed) = parse_tz_rule(bytes, pos)?;
        let _ = consumed;
        (Some(start), Some(end))
    } else {
        (None, None)
    };

    Some(PosixRule { std_name, std_offset, dst_name, dst_offset, dst_start, dst_end })
}

/// Parse a timezone name (`<...>` or alphabetic sequence).
fn parse_tz_name(bytes: &[u8], pos: usize) -> Option<(String, usize)> {
    if pos >= bytes.len() { return None; }
    if bytes[pos] == b'<' {
        // Quoted name.
        let end = bytes[pos+1..].iter().position(|&b| b == b'>')?;
        let name = core::str::from_utf8(&bytes[pos+1..pos+1+end]).ok()?;
        return Some((String::from(name), end + 2));
    }
    // Unquoted alphabetic name (POSIX: at least 3 chars; we accept 1+).
    let mut end = pos;
    while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
        end += 1;
    }
    if end == pos { return None; }
    let name = core::str::from_utf8(&bytes[pos..end]).ok()?;
    Some((String::from(name), end - pos))
}

/// Parse a POSIX timezone offset `[+-]hh[:mm[:ss]]`.
///
/// Returns the offset in seconds (POSIX convention: positive = west of UTC).
fn parse_tz_offset(bytes: &[u8], pos: usize) -> Option<(i32, usize)> {
    if pos >= bytes.len() { return None; }
    let (sign, mut p) = if bytes[pos] == b'-' {
        (-1i32, pos + 1)
    } else if bytes[pos] == b'+' {
        (1i32, pos + 1)
    } else {
        (1i32, pos)
    };

    let (hh, consumed) = parse_decimal(bytes, p)?;
    p += consumed;
    let mut secs = hh * 3600i32;

    if p < bytes.len() && bytes[p] == b':' {
        p += 1;
        if let Some((mm, c)) = parse_decimal(bytes, p) {
            secs += mm * 60;
            p += c;
            if p < bytes.len() && bytes[p] == b':' {
                p += 1;
                if let Some((ss, c)) = parse_decimal(bytes, p) {
                    secs += ss;
                    p += c;
                }
            }
        }
    }

    Some((sign * secs, p - pos))
}

/// Parse a POSIX transition rule `Jn`, `n`, or `Mm.w.d[/time]`.
fn parse_tz_rule(bytes: &[u8], pos: usize) -> Option<(PosixTransition, usize)> {
    if pos >= bytes.len() { return None; }
    let (kind, mut p) = if bytes[pos] == b'J' {
        // Julian day 1–365 (no leap day).
        let (n, c) = parse_decimal(&bytes, pos + 1)?;
        (TransitionKind::Julian(n as u16), pos + 1 + c)
    } else if bytes[pos] == b'M' {
        // Mm.w.d
        let (month, c1) = parse_decimal(bytes, pos + 1)?;
        let p1 = pos + 1 + c1;
        if p1 >= bytes.len() || bytes[p1] != b'.' { return None; }
        let (week, c2) = parse_decimal(bytes, p1 + 1)?;
        let p2 = p1 + 1 + c2;
        if p2 >= bytes.len() || bytes[p2] != b'.' { return None; }
        let (day, c3) = parse_decimal(bytes, p2 + 1)?;
        let p3 = p2 + 1 + c3;
        (TransitionKind::MonthWeekDay {
            month: month as u8,
            week:  week as u8,
            day:   day as u8,
        }, p3)
    } else {
        // Zero-based Julian day.
        let (n, c) = parse_decimal(bytes, pos)?;
        (TransitionKind::Day(n as u16), pos + c)
    };

    // Optional /time suffix (seconds from midnight, default 02:00:00 = 7200).
    let (time, consumed_t) = if p < bytes.len() && bytes[p] == b'/' {
        let (off, c) = parse_tz_offset(bytes, p + 1)?;
        // Note: /time uses wall-clock time in local standard time; store as-is.
        (-off, c + 1) // sign flip: /time is given as east here, we store corrected
    } else {
        (7200i32, 0)
    };
    p += consumed_t;

    Some((PosixTransition { kind, time }, p - pos))
}

/// Parse a non-negative decimal integer from `bytes[pos..]`.
fn parse_decimal(bytes: &[u8], pos: usize) -> Option<(i32, usize)> {
    let mut p = pos;
    while p < bytes.len() && bytes[p] >= b'0' && bytes[p] <= b'9' {
        p += 1;
    }
    if p == pos { return None; }
    let s = core::str::from_utf8(&bytes[pos..p]).ok()?;
    let mut v: i32 = 0;
    for b in s.bytes() {
        v = v.checked_mul(10)?.checked_add((b - b'0') as i32)?;
    }
    Some((v, p - pos))
}

// ---------------------------------------------------------------------------
// PosixRule UTC offset / DST lookup
// ---------------------------------------------------------------------------

impl PosixRule {
    fn utc_offset_at(&self, unix_seconds: i64) -> i32 {
        if self.is_dst_at(unix_seconds) { self.dst_offset } else { self.std_offset }
    }

    fn is_dst_at(&self, unix_seconds: i64) -> bool {
        let (start, end) = match (&self.dst_start, &self.dst_end) {
            (Some(s), Some(e)) => (s, e),
            _ => return false,
        };

        // Get the UTC year for this timestamp.
        let days = unix_seconds / 86400;
        let year = unix_days_to_year(days);

        let start_unix = transition_to_unix(start, year, self.std_offset);
        let end_unix   = transition_to_unix(end,   year, self.dst_offset);

        if start_unix <= end_unix {
            // Northern hemisphere: DST is active between start and end.
            unix_seconds >= start_unix && unix_seconds < end_unix
        } else {
            // Southern hemisphere: DST active before end or after start.
            unix_seconds >= start_unix || unix_seconds < end_unix
        }
    }

    fn abbreviation_at(&self, unix_seconds: i64) -> &str {
        if self.is_dst_at(unix_seconds) {
            self.dst_name.as_str()
        } else {
            self.std_name.as_str()
        }
    }
}

/// Compute the Unix timestamp of a POSIX transition in the given year.
///
/// `base_offset` is the UTC offset (seconds east) that is active just before the transition.
fn transition_to_unix(trans: &PosixTransition, year: i32, base_offset: i32) -> i64 {
    let jan1_unix = year_to_jan1_unix(year);
    let day_of_year = transition_day_of_year(&trans.kind, year);
    // Wall-clock time relative to midnight local standard time.
    let midnight_unix = jan1_unix + day_of_year as i64 * 86400;
    // trans.time is already in seconds (stored east); subtract local offset to get UTC.
    midnight_unix + trans.time as i64 - base_offset as i64
}

/// Compute the day-of-year (0-based) for a transition rule in the given year.
fn transition_day_of_year(kind: &TransitionKind, year: i32) -> u16 {
    match kind {
        TransitionKind::Julian(n) => {
            // Julian day 1–365; Feb 29 is not counted even in leap years.
            // Day 1 = Jan 1 = day-of-year 0.
            (n - 1) as u16
        }
        TransitionKind::Day(n) => *n,
        TransitionKind::MonthWeekDay { month, week, day } => {
            // Find the first `day`-of-week in `month`, then add (week-1) full weeks.
            // If week==5 it means the last occurrence.
            let m = *month as u8;
            let w = *week as u8;
            let d = *day as u8; // 0=Sun

            // Day-of-year for the 1st of the month (0-based Jan 1 = 0).
            let mut doy: u16 = 0;
            for mi in 1..m {
                doy += days_in_month_for_year(year, mi) as u16;
            }

            // Jan 1 of `year` weekday.
            let jan1_unix = year_to_jan1_unix(year);
            let jan1_wd   = (((jan1_unix / 86400) + 4) % 7) as u8; // 0=Sun

            // Day-of-week of the 1st of month m.
            let month1_wd = ((jan1_wd as u16 + doy) % 7) as u8;

            // Offset to the first `d`-weekday in the month.
            let delta = (d + 7 - month1_wd) % 7;
            let first_occurrence = doy + delta as u16;

            if w == 5 {
                // Last occurrence: add weeks until we go past the end of the month.
                let month_len = days_in_month_for_year(year, m) as u16;
                let mut occ = first_occurrence;
                while occ + 7 < doy + month_len {
                    occ += 7;
                }
                occ
            } else {
                first_occurrence + (w as u16 - 1) * 7
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar helpers
// ---------------------------------------------------------------------------

fn days_in_month_for_year(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11              => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Unix timestamp of January 1, 00:00:00 UTC for the given year.
fn year_to_jan1_unix(year: i32) -> i64 {
    // Hinnant algorithm inverse: compute days from epoch to Jan 1 of year.
    let y = year as i64 - 1;
    let era  = if y >= 0 { y } else { y - 399 } / 400;
    let yoe  = (y - era * 400) as u64;
    let doe  = 365 * yoe + yoe / 4 - yoe / 100; // day of era for Jan 1 (off by 59 from Mar 1 era)
    // The Hinnant epoch is 0000-03-01; Jan 1 of year `y+1` is at doy=306 in
    // the shifted calendar (months shifted by 10).
    // Simpler: just count days directly.
    let leap_days = (year as i64 - 1) / 4 - (year as i64 - 1) / 100 + (year as i64 - 1) / 400;
    let days = (year as i64 - 1970) * 365 + leap_days - (1970 / 4 - 1970 / 100 + 1970 / 400);
    let _ = (era, yoe, doe);
    days * 86400
}

/// Approximate year from number of days since Unix epoch.
fn unix_days_to_year(days: i64) -> i32 {
    // Approximate: 365.2425 days/year; refine below.
    let approx = (days * 400 / 146097 + 1970) as i32;
    // Adjust by at most 1.
    if year_to_jan1_unix(approx + 1) / 86400 <= days {
        approx + 1
    } else if year_to_jan1_unix(approx) / 86400 > days {
        approx - 1
    } else {
        approx
    }
}

// ---------------------------------------------------------------------------
// Big-endian read helpers
// ---------------------------------------------------------------------------

#[inline]
fn u32_be(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

#[inline]
fn i32_be(b: &[u8]) -> i32 {
    u32_be(b) as i32
}

#[inline]
fn i64_be(b: &[u8]) -> i64 {
    let hi = u32_be(&b[0..4]) as i64;
    let lo = u32_be(&b[4..8]) as i64;
    (hi << 32) | lo
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

fn push_u8_2(s: &mut String, value: u8) {
    if value < 10 { s.push('0'); }
    let mut buf = [0u8; 3];
    let mut cursor = 3usize;
    let mut v = value;
    if v == 0 {
        cursor -= 1;
        buf[cursor] = b'0';
    } else {
        while v > 0 {
            cursor -= 1;
            buf[cursor] = b'0' + v % 10;
            v /= 10;
        }
    }
    if let Ok(st) = core::str::from_utf8(&buf[cursor..]) {
        s.push_str(st);
    }
}
