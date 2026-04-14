//! Time — Duration, Instant, SystemTime, and the Time facade.

use crate::raw;
use alloc::string::String;

const NANOS_PER_SEC:   u64 = 1_000_000_000;
const NANOS_PER_MILLI: u64 = 1_000_000;
const NANOS_PER_MICRO: u64 = 1_000;

// Clock IDs matching the kernel.
const CLOCK_REALTIME:  i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;

// ---------------------------------------------------------------------------
// Duration
// ---------------------------------------------------------------------------

/// A span of time, stored as nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    nanos: u64,
}

impl Duration {
    pub const ZERO: Duration = Duration { nanos: 0 };

    pub fn ms(millis: u64) -> Duration {
        Duration { nanos: millis * NANOS_PER_MILLI }
    }

    pub fn secs(seconds: u64) -> Duration {
        Duration { nanos: seconds * NANOS_PER_SEC }
    }

    pub fn mins(minutes: u64) -> Duration {
        Duration { nanos: minutes * 60 * NANOS_PER_SEC }
    }

    pub fn hours(hours: u64) -> Duration {
        Duration { nanos: hours * 3600 * NANOS_PER_SEC }
    }

    pub fn as_nanos(&self) -> u64 {
        self.nanos
    }

    pub fn as_millis(&self) -> u64 {
        self.nanos / NANOS_PER_MILLI
    }

    pub fn as_secs(&self) -> u64 {
        self.nanos / NANOS_PER_SEC
    }

    /// Integer approximation of seconds (no f64 in no_std).
    pub fn as_secs_f64_approx(&self) -> u64 {
        self.as_secs()
    }
}

impl core::ops::Add for Duration {
    type Output = Duration;
    fn add(self, rhs: Duration) -> Duration {
        Duration { nanos: self.nanos + rhs.nanos }
    }
}

impl core::ops::Sub for Duration {
    type Output = Duration;
    fn sub(self, rhs: Duration) -> Duration {
        Duration { nanos: self.nanos.saturating_sub(rhs.nanos) }
    }
}

// ---------------------------------------------------------------------------
// Instant — monotonic
// ---------------------------------------------------------------------------

/// A monotonic timestamp (nanoseconds since boot).
#[derive(Clone, Copy, Debug)]
pub struct Instant {
    nanos: u64,
}

impl Instant {
    fn read_monotonic() -> u64 {
        let mut buf = [0u64; 2];
        let result = raw::raw_clock_gettime(CLOCK_MONOTONIC, buf.as_mut_ptr());
        if result < 0 {
            return 0;
        }
        buf[0] * NANOS_PER_SEC + buf[1]
    }

    /// Capture the current monotonic time.
    pub fn now() -> Instant {
        Instant { nanos: Self::read_monotonic() }
    }

    /// Elapsed time since this Instant was captured.
    pub fn elapsed(&self) -> Duration {
        let current = Self::read_monotonic();
        Duration { nanos: current.saturating_sub(self.nanos) }
    }
}

// ---------------------------------------------------------------------------
// SystemTime — wall clock
// ---------------------------------------------------------------------------

/// A real-time (wall clock) timestamp.
#[derive(Clone, Copy, Debug)]
pub struct SystemTime {
    pub secs: u64,
    pub nanos: u64,
}

impl SystemTime {
    /// Read the real-time clock.
    pub fn now() -> SystemTime {
        let mut buf = [0u64; 2];
        let result = raw::raw_clock_gettime(CLOCK_REALTIME, buf.as_mut_ptr());
        if result < 0 {
            return SystemTime { secs: 0, nanos: 0 };
        }
        SystemTime { secs: buf[0], nanos: buf[1] }
    }

    /// Unix timestamp (seconds since epoch).
    pub fn unix_timestamp(&self) -> u64 {
        self.secs
    }

    /// Format as "YYYY-MM-DD HH:MM:SS" (UTC).
    ///
    /// Implements Gregorian calendar conversion from Unix timestamp.
    pub fn format_iso8601(&self) -> String {
        let unix_seconds = self.secs;

        // Days since 1970-01-01
        let days = unix_seconds / 86400;
        let remaining_seconds = unix_seconds % 86400;

        let hour   = remaining_seconds / 3600;
        let minute = (remaining_seconds % 3600) / 60;
        let second = remaining_seconds % 60;

        // Gregorian calendar: 400-year cycle = 146097 days
        // Shift epoch to 0000-03-01 for simplicity.
        // Using algorithm from http://howardhinnant.github.io/date_algorithms.html
        let z = days as i64 + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let day_of_era = (z - era * 146097) as u64;
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
        let year = year_of_era as i64 + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day   = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
        let year  = if month <= 2 { year + 1 } else { year };

        // Format manually without format! floats.
        let mut buf = [0u8; 19];
        write_u64_padded(&mut buf[0..4], year as u64, 4);
        buf[4] = b'-';
        write_u64_padded(&mut buf[5..7], month, 2);
        buf[7] = b'-';
        write_u64_padded(&mut buf[8..10], day, 2);
        buf[10] = b' ';
        write_u64_padded(&mut buf[11..13], hour, 2);
        buf[13] = b':';
        write_u64_padded(&mut buf[14..16], minute, 2);
        buf[16] = b':';
        write_u64_padded(&mut buf[17..19], second, 2);

        core::str::from_utf8(&buf).unwrap_or("0000-00-00 00:00:00").into()
    }
}

fn write_u64_padded(buf: &mut [u8], value: u64, width: usize) {
    let mut digits = [b'0'; 10];
    let mut remaining = value;
    let mut position = 0;
    loop {
        digits[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        position += 1;
        if remaining == 0 {
            break;
        }
    }
    // Write into buf right-to-left, zero-padded.
    for i in 0..width {
        let digit_index = if i < position { position - 1 - i } else { 0 };
        let digit = if i < position { digits[digit_index] } else { b'0' };
        buf[width - 1 - i] = digit;
    }
}

// ---------------------------------------------------------------------------
// Time facade
// ---------------------------------------------------------------------------

pub struct Time;

impl Time {
    /// Current monotonic timestamp.
    pub fn now() -> Instant {
        Instant::now()
    }

    /// Current wall-clock time.
    pub fn system_now() -> SystemTime {
        SystemTime::now()
    }

    /// Unix timestamp (seconds since epoch).
    pub fn unix_timestamp() -> u64 {
        SystemTime::now().unix_timestamp()
    }

    /// Timezone string — always "UTC" in v1.0.
    pub fn timezone() -> &'static str {
        "UTC"
    }

    /// Sleep for the given duration.
    pub fn sleep(duration: Duration) -> Result<(), i32> {
        let secs  = duration.as_secs();
        let nanos = duration.as_nanos() % NANOS_PER_SEC;
        let timespec = [secs, nanos];
        let result = raw::raw_nanosleep(timespec.as_ptr());
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// DateTime — full calendar type analogous to C# System.DateTime
// ---------------------------------------------------------------------------

/// A calendar date and time in UTC.
///
/// Analogous to C#'s `System.DateTime` with `Kind = DateTimeKind.Utc`.
/// Arithmetic is performed on the internal Unix timestamp; calendar fields
/// are derived on demand via the Hinnant algorithm.
///
/// # Examples
///
/// ```no_run
/// let now      = DateTime::now();
/// let tomorrow = now.add_days(1);
/// let elapsed  = tomorrow - now;           // Duration
/// let leap     = DateTime::is_leap_year(2024); // true
/// let parsed   = DateTime::parse("2025-06-15 12:30:00");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    /// Year (e.g. 2025).
    pub year:       i32,
    /// Month 1–12.
    pub month:      u8,
    /// Day of month 1–31.
    pub day:        u8,
    /// Hour 0–23.
    pub hour:       u8,
    /// Minute 0–59.
    pub minute:     u8,
    /// Second 0–59.
    pub second:     u8,
    /// Nanosecond 0–999_999_999.
    pub nanosecond: u32,
    /// Day of week: 0 = Sunday, 1 = Monday, …, 6 = Saturday.
    pub weekday:    u8,
    /// Cached Unix timestamp in seconds (1970-01-01 00:00:00 UTC = 0).
    unix_seconds:   u64,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl DateTime {
    /// Decompose a Unix timestamp into UTC calendar fields.
    ///
    /// Uses the Hinnant algorithm:
    /// <http://howardhinnant.github.io/date_algorithms.html>
    pub fn from_unix(unix_seconds: u64, nanosecond: u32) -> DateTime {
        let days_since_epoch  = unix_seconds / 86400;
        let remaining_seconds = unix_seconds % 86400;

        let hour   = (remaining_seconds / 3600) as u8;
        let minute = ((remaining_seconds % 3600) / 60) as u8;
        let second = (remaining_seconds % 60) as u8;

        // 1970-01-01 was a Thursday → weekday index 4 if Sun=0.
        let weekday = ((days_since_epoch + 4) % 7) as u8;

        let z           = days_since_epoch as i64 + 719468;
        let era         = if z >= 0 { z } else { z - 146096 } / 146097;
        let day_of_era  = (z - era * 146097) as u64;
        let year_of_era = (day_of_era
            - day_of_era / 1460
            + day_of_era / 36524
            - day_of_era / 146096) / 365;
        let year_raw    = year_of_era as i64 + era * 400;
        let day_of_year = day_of_era
            - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day         = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u8;
        let month       = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 } as u8;
        let year        = (if month <= 2 { year_raw + 1 } else { year_raw }) as i32;

        DateTime { year, month, day, hour, minute, second, nanosecond, weekday, unix_seconds }
    }

    /// Build from individual UTC components.  Returns `None` if any field is
    /// out of range.
    ///
    /// Analogous to `new DateTime(year, month, day, hour, minute, second)` in C#.
    pub fn new(
        year: i32, month: u8, day: u8,
        hour: u8,  minute: u8, second: u8,
    ) -> Option<DateTime> {
        if month < 1 || month > 12 { return None; }
        if day   < 1 || day > DateTime::days_in_month(year, month) { return None; }
        if hour > 23 || minute > 59 || second > 59 { return None; }
        Some(DateTime::from_unix(
            dt_to_unix(year, month, day, hour, minute, second), 0,
        ))
    }

    /// Current UTC wall-clock time (`DateTime.UtcNow` in C#).
    pub fn now() -> DateTime {
        let st = SystemTime::now();
        DateTime::from_unix(st.secs, st.nanos as u32)
    }

    /// Alias for `now()` — Bazzulto has no local timezone.
    #[inline]
    pub fn utc_now() -> DateTime { DateTime::now() }

    /// Today at midnight UTC (`DateTime.Today` in C#).
    pub fn today() -> DateTime {
        let st = SystemTime::now();
        DateTime::from_unix((st.secs / 86400) * 86400, 0)
    }

    /// Unix epoch constant: 1970-01-01 00:00:00 UTC.
    pub const fn unix_epoch() -> DateTime {
        DateTime {
            year: 1970, month: 1, day: 1,
            hour: 0, minute: 0, second: 0,
            nanosecond: 0, weekday: 4,
            unix_seconds: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

impl DateTime {
    /// Day of the year, 1-based (January 1 = 1).
    ///
    /// Analogous to `DateTime.DayOfYear` in C#.
    pub fn day_of_year(&self) -> u16 {
        let mut doy = self.day as u16;
        for m in 1..self.month {
            doy += DateTime::days_in_month(self.year, m) as u16;
        }
        doy
    }

    /// Millisecond component (0–999), derived from `nanosecond`.
    ///
    /// Analogous to `DateTime.Millisecond` in C#.
    pub fn millisecond(&self) -> u16 {
        (self.nanosecond / 1_000_000) as u16
    }

    /// Microsecond component (0–999_999), derived from `nanosecond`.
    pub fn microsecond(&self) -> u32 {
        self.nanosecond / 1_000
    }

    /// Seconds since 1970-01-01 00:00:00 UTC.
    ///
    /// Analogous to `DateTimeOffset.ToUnixTimeSeconds()` in C#.
    pub fn to_unix_time_seconds(&self) -> u64 { self.unix_seconds }

    /// Milliseconds since 1970-01-01 00:00:00 UTC.
    ///
    /// Analogous to `DateTimeOffset.ToUnixTimeMilliseconds()` in C#.
    pub fn to_unix_time_milliseconds(&self) -> u64 {
        self.unix_seconds * 1000 + (self.nanosecond / 1_000_000) as u64
    }

    /// 100-nanosecond ticks since 0001-01-01 00:00:00 UTC.
    ///
    /// Analogous to `DateTime.Ticks` in C#.
    /// (1970-01-01 = tick 621_355_968_000_000_000)
    pub fn ticks(&self) -> u64 {
        const TICKS_AT_EPOCH: u64 = 621_355_968_000_000_000;
        TICKS_AT_EPOCH
            + self.unix_seconds * 10_000_000
            + (self.nanosecond / 100) as u64
    }

    /// `true` if the time-of-day is exactly midnight (00:00:00.000000000).
    pub fn is_midnight(&self) -> bool {
        self.hour == 0 && self.minute == 0 && self.second == 0 && self.nanosecond == 0
    }
}

// ---------------------------------------------------------------------------
// Calendar helpers (static)
// ---------------------------------------------------------------------------

impl DateTime {
    /// `true` if `year` is a Gregorian leap year.
    ///
    /// Analogous to `DateTime.IsLeapYear(year)` in C#.
    pub fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    /// Number of days in the given month of the given year (1–28/29/30/31).
    ///
    /// Analogous to `DateTime.DaysInMonth(year, month)` in C#.
    pub fn days_in_month(year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11              => 30,
            2 => if DateTime::is_leap_year(year) { 29 } else { 28 },
            _ => 0,
        }
    }

    /// Number of days in the year (365 or 366).
    pub fn days_in_year(year: i32) -> u16 {
        if DateTime::is_leap_year(year) { 366 } else { 365 }
    }
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

impl DateTime {
    /// Add whole seconds. Negative values subtract.
    pub fn add_seconds(&self, seconds: i64) -> DateTime {
        let new_unix = if seconds >= 0 {
            self.unix_seconds.saturating_add(seconds as u64)
        } else {
            self.unix_seconds.saturating_sub((-seconds) as u64)
        };
        DateTime::from_unix(new_unix, self.nanosecond)
    }

    /// Add whole minutes.
    pub fn add_minutes(&self, minutes: i64) -> DateTime {
        self.add_seconds(minutes.saturating_mul(60))
    }

    /// Add whole hours.
    pub fn add_hours(&self, hours: i64) -> DateTime {
        self.add_seconds(hours.saturating_mul(3600))
    }

    /// Add whole days.
    pub fn add_days(&self, days: i64) -> DateTime {
        self.add_seconds(days.saturating_mul(86400))
    }

    /// Add whole weeks.
    pub fn add_weeks(&self, weeks: i64) -> DateTime {
        self.add_days(weeks.saturating_mul(7))
    }

    /// Add calendar months, clamping the day if the target month is shorter.
    ///
    /// Analogous to `DateTime.AddMonths(n)` in C#.
    pub fn add_months(&self, months: i32) -> DateTime {
        let total = (self.year * 12 + self.month as i32 - 1) + months;
        let new_year  = total.div_euclid(12);
        let new_month = (total.rem_euclid(12) + 1) as u8;
        let new_day   = self.day.min(DateTime::days_in_month(new_year, new_month));
        DateTime::from_unix(
            dt_to_unix(new_year, new_month, new_day, self.hour, self.minute, self.second),
            self.nanosecond,
        )
    }

    /// Add calendar years.  Feb 29 on leap years is clamped to Feb 28.
    ///
    /// Analogous to `DateTime.AddYears(n)` in C#.
    pub fn add_years(&self, years: i32) -> DateTime {
        self.add_months(years.saturating_mul(12))
    }

    /// Add a `Duration`.
    ///
    /// Analogous to `DateTime.Add(TimeSpan)` in C#.
    pub fn add(&self, duration: Duration) -> DateTime {
        let extra_secs  = duration.nanos / NANOS_PER_SEC;
        let extra_nanos = (duration.nanos % NANOS_PER_SEC) as u32;
        let mut new_nanos = self.nanosecond + extra_nanos;
        let carry = new_nanos / 1_000_000_000;
        new_nanos %= 1_000_000_000;
        DateTime::from_unix(
            self.unix_seconds.saturating_add(extra_secs).saturating_add(carry as u64),
            new_nanos,
        )
    }

    /// Elapsed time between `self` and `other` (`self - other`).
    ///
    /// Returns `Duration::ZERO` if `other` is after `self`.
    /// Analogous to `DateTime.Subtract(DateTime)` in C#.
    pub fn subtract(&self, other: &DateTime) -> Duration {
        if self.unix_seconds < other.unix_seconds { return Duration::ZERO; }
        let secs = self.unix_seconds - other.unix_seconds;
        let nanos = if self.nanosecond >= other.nanosecond {
            secs * NANOS_PER_SEC + (self.nanosecond - other.nanosecond) as u64
        } else {
            (secs - 1) * NANOS_PER_SEC
                + 1_000_000_000
                + self.nanosecond as u64
                - other.nanosecond as u64
        };
        Duration { nanos }
    }

    /// Return a copy with the time-of-day replaced.  Returns `None` if out of
    /// range.
    pub fn with_time(&self, hour: u8, minute: u8, second: u8) -> Option<DateTime> {
        if hour > 23 || minute > 59 || second > 59 { return None; }
        Some(DateTime::from_unix(
            dt_to_unix(self.year, self.month, self.day, hour, minute, second),
            self.nanosecond,
        ))
    }

    /// Return a copy with the nanosecond replaced.  Returns `None` if ≥ 1e9.
    pub fn with_nanosecond(&self, nanosecond: u32) -> Option<DateTime> {
        if nanosecond >= 1_000_000_000 { return None; }
        Some(DateTime::from_unix(self.unix_seconds, nanosecond))
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &DateTime) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateTime {
    fn cmp(&self, other: &DateTime) -> core::cmp::Ordering {
        self.unix_seconds.cmp(&other.unix_seconds)
            .then(self.nanosecond.cmp(&other.nanosecond))
    }
}

// ---------------------------------------------------------------------------
// Operator overloads
// ---------------------------------------------------------------------------

impl core::ops::Sub for DateTime {
    type Output = Duration;
    /// `lhs - rhs` → elapsed `Duration`.  Returns `Duration::ZERO` if rhs > lhs.
    fn sub(self, rhs: DateTime) -> Duration { self.subtract(&rhs) }
}

impl core::ops::Add<Duration> for DateTime {
    type Output = DateTime;
    fn add(self, rhs: Duration) -> DateTime { DateTime::add(&self, rhs) }
}

impl core::ops::Sub<Duration> for DateTime {
    type Output = DateTime;
    fn sub(self, rhs: Duration) -> DateTime {
        let secs  = rhs.nanos / NANOS_PER_SEC;
        let nanos = (rhs.nanos % NANOS_PER_SEC) as u32;
        let unix  = self.unix_seconds.saturating_sub(secs);
        let (unix, new_nanos) = if self.nanosecond >= nanos {
            (unix, self.nanosecond - nanos)
        } else {
            (unix.saturating_sub(1), 1_000_000_000 + self.nanosecond - nanos)
        };
        DateTime::from_unix(unix, new_nanos)
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

impl DateTime {
    /// Abbreviated weekday name ("Sun", "Mon", …, "Sat").
    pub fn weekday_name_short(&self) -> &'static str {
        ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"]
            .get(self.weekday as usize).copied().unwrap_or("???")
    }

    /// Full weekday name ("Sunday", "Monday", …, "Saturday").
    pub fn weekday_name_long(&self) -> &'static str {
        ["Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"]
            .get(self.weekday as usize).copied().unwrap_or("???")
    }

    /// Abbreviated month name ("Jan", "Feb", …, "Dec").
    pub fn month_name_short(&self) -> &'static str {
        const N: [&str; 12] = [
            "Jan","Feb","Mar","Apr","May","Jun",
            "Jul","Aug","Sep","Oct","Nov","Dec",
        ];
        if self.month >= 1 && self.month <= 12 { N[(self.month-1) as usize] } else { "???" }
    }

    /// Full month name ("January", "February", …, "December").
    pub fn month_name_long(&self) -> &'static str {
        const N: [&str; 12] = [
            "January","February","March","April","May","June",
            "July","August","September","October","November","December",
        ];
        if self.month >= 1 && self.month <= 12 { N[(self.month-1) as usize] } else { "???" }
    }

    /// `"YYYY-MM-DD"` — date only.
    pub fn to_date_string(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        dt_push_i32_4(&mut s, self.year);
        s.push('-'); dt_push_u8_2(&mut s, self.month);
        s.push('-'); dt_push_u8_2(&mut s, self.day);
        s
    }

    /// `"HH:MM:SS"` — time only.
    pub fn to_time_string(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        dt_push_u8_2(&mut s, self.hour);
        s.push(':'); dt_push_u8_2(&mut s, self.minute);
        s.push(':'); dt_push_u8_2(&mut s, self.second);
        s
    }

    /// `"YYYY-MM-DD HH:MM:SS"` — ISO 8601, UTC.
    ///
    /// Analogous to `DateTime.ToString("yyyy-MM-dd HH:mm:ss")` in C#.
    pub fn format_iso8601(&self) -> alloc::string::String {
        let mut s = self.to_date_string();
        s.push(' ');
        s.push_str(&self.to_time_string());
        s
    }

    /// `"YYYY-MM-DDTHH:MM:SSZ"` — RFC 3339.
    pub fn to_rfc3339(&self) -> alloc::string::String {
        let mut s = self.to_date_string();
        s.push('T');
        s.push_str(&self.to_time_string());
        s.push('Z');
        s
    }

    /// `"Thu Jan  1 00:00:00 UTC 1970"` — POSIX `date` format.
    pub fn format_posix_date(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        s.push_str(self.weekday_name_short()); s.push(' ');
        s.push_str(self.month_name_short());   s.push(' ');
        if self.day < 10 { s.push(' '); }
        dt_push_u8_raw(&mut s, self.day);      s.push(' ');
        dt_push_u8_2(&mut s, self.hour);       s.push(':');
        dt_push_u8_2(&mut s, self.minute);     s.push(':');
        dt_push_u8_2(&mut s, self.second);
        s.push_str(" UTC ");
        dt_push_i32_4(&mut s, self.year);
        s
    }

    /// Apply a strftime-style format string.
    ///
    /// Supported: `%Y %y %m %d %H %M %S %f %j %A %a %B %b %h %Z %z %n %t %%`
    ///
    /// Analogous to `DateTime.ToString(format)` in C# with custom format strings.
    pub fn format(&self, fmt: &str) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        let bytes = fmt.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 1 < bytes.len() {
                i += 1;
                match bytes[i] {
                    b'Y' => dt_push_i32_4(&mut result, self.year),
                    b'y' => dt_push_u8_2(&mut result, (self.year % 100).unsigned_abs() as u8),
                    b'm' => dt_push_u8_2(&mut result, self.month),
                    b'd' => dt_push_u8_2(&mut result, self.day),
                    b'H' => dt_push_u8_2(&mut result, self.hour),
                    b'M' => dt_push_u8_2(&mut result, self.minute),
                    b'S' => dt_push_u8_2(&mut result, self.second),
                    b'f' => dt_push_u32_6(&mut result, self.nanosecond / 1000),
                    b'j' => dt_push_u16_3(&mut result, self.day_of_year()),
                    b'A' => result.push_str(self.weekday_name_long()),
                    b'a' => result.push_str(self.weekday_name_short()),
                    b'B' => result.push_str(self.month_name_long()),
                    b'b' | b'h' => result.push_str(self.month_name_short()),
                    b'Z' => result.push_str("UTC"),
                    b'z' => result.push_str("+0000"),
                    b'n' => result.push('\n'),
                    b't' => result.push('\t'),
                    b'%' => result.push('%'),
                    other => { result.push('%'); result.push(other as char); }
                }
            } else {
                result.push(bytes[i] as char);
            }
            i += 1;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl DateTime {
    /// Parse `"YYYY-MM-DD"` or `"YYYY-MM-DD HH:MM:SS"` (UTC assumed).
    ///
    /// Returns `None` on malformed input or out-of-range fields.
    /// Analogous to `DateTime.Parse(s)` in C#.
    pub fn parse(s: &str) -> Option<DateTime> {
        let b = s.as_bytes();
        if b.len() < 10 { return None; }
        let year  = dt_parse_u16(&b[0..4])? as i32;
        if b[4] != b'-' { return None; }
        let month = dt_parse_u8_2(&b[5..7])?;
        if b[7] != b'-' { return None; }
        let day   = dt_parse_u8_2(&b[8..10])?;
        let (hour, minute, second) = if b.len() >= 19 {
            let sep = b[10];
            if sep != b' ' && sep != b'T' { return None; }
            let h = dt_parse_u8_2(&b[11..13])?;
            if b[13] != b':' { return None; }
            let m = dt_parse_u8_2(&b[14..16])?;
            if b[16] != b':' { return None; }
            let s = dt_parse_u8_2(&b[17..19])?;
            (h, m, s)
        } else {
            (0, 0, 0)
        };
        DateTime::new(year, month, day, hour, minute, second)
    }

    /// Parse a decimal Unix timestamp string (seconds since epoch).
    pub fn from_unix_str(s: &str) -> Option<DateTime> {
        let mut v: u64 = 0;
        for b in s.bytes() {
            if b < b'0' || b > b'9' { return None; }
            v = v.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        }
        Some(DateTime::from_unix(v, 0))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers — dt_ prefix avoids name collisions with outer helpers
// ---------------------------------------------------------------------------

/// Convert UTC calendar fields to a Unix timestamp in seconds (Hinnant inverse).
fn dt_to_unix(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> u64 {
    let m = month as i64;
    let y = if m <= 2 { year as i64 - 1 } else { year as i64 };
    let era        = if y >= 0 { y } else { y - 399 } / 400;
    let yoe        = (y - era * 400) as u64;
    let mop        = (if m > 2 { m - 3 } else { m + 9 }) as u64;
    let doy        = (153 * mop + 2) / 5 + day as u64 - 1;
    let doe        = 365 * yoe + yoe / 4 - yoe / 100 + doy;
    let days       = (era * 146097 + doe as i64 - 719468).max(0) as u64;
    days * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64
}

fn dt_parse_u16(b: &[u8]) -> Option<u16> {
    let mut v: u16 = 0;
    for &c in b {
        if c < b'0' || c > b'9' { return None; }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u16)?;
    }
    Some(v)
}

fn dt_parse_u8_2(b: &[u8]) -> Option<u8> {
    if b.len() < 2 { return None; }
    if b[0] < b'0' || b[0] > b'9' || b[1] < b'0' || b[1] > b'9' { return None; }
    Some((b[0] - b'0') * 10 + (b[1] - b'0'))
}

fn dt_push_u8_2(s: &mut alloc::string::String, v: u8) {
    if v < 10 { s.push('0'); }
    dt_push_u8_raw(s, v);
}

fn dt_push_u8_raw(s: &mut alloc::string::String, v: u8) {
    let mut buf = [0u8; 3];
    let mut cur = 3usize;
    let mut n = v;
    if n == 0 { cur -= 1; buf[cur] = b'0'; }
    else { while n > 0 { cur -= 1; buf[cur] = b'0' + n % 10; n /= 10; } }
    if let Ok(st) = core::str::from_utf8(&buf[cur..]) { s.push_str(st); }
}

fn dt_push_i32_4(s: &mut alloc::string::String, v: i32) {
    if v < 0 { s.push('-'); }
    let mut buf = [0u8; 10];
    let mut cur = 10usize;
    let mut n = v.unsigned_abs();
    if n == 0 { cur -= 1; buf[cur] = b'0'; }
    else { while n > 0 { cur -= 1; buf[cur] = b'0' + (n % 10) as u8; n /= 10; } }
    let digits = 10 - cur;
    for _ in digits..4 { s.push('0'); }
    if let Ok(st) = core::str::from_utf8(&buf[cur..]) { s.push_str(st); }
}

fn dt_push_u32_6(s: &mut alloc::string::String, v: u32) {
    let mut buf = [b'0'; 6];
    let mut cur = 6usize;
    let mut n = v;
    while n > 0 && cur > 0 { cur -= 1; buf[cur] = b'0' + (n % 10) as u8; n /= 10; }
    if let Ok(st) = core::str::from_utf8(&buf) { s.push_str(st); }
}

fn dt_push_u16_3(s: &mut alloc::string::String, v: u16) {
    let mut buf = [b'0'; 3];
    let mut cur = 3usize;
    let mut n = v;
    while n > 0 && cur > 0 { cur -= 1; buf[cur] = b'0' + (n % 10) as u8; n /= 10; }
    if let Ok(st) = core::str::from_utf8(&buf) { s.push_str(st); }
}

// ---------------------------------------------------------------------------
// Legacy types kept for callers that used the old API
// ---------------------------------------------------------------------------

/// Clock identifiers (POSIX-compatible).
#[repr(i32)]
#[derive(Clone, Copy, Debug)]
pub enum ClockId {
    Realtime  = 0,
    Monotonic = 1,
}

/// POSIX `struct timespec` — seconds + nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeSpec {
    pub seconds:     u64,
    pub nanoseconds: u64,
}

impl TimeSpec {
    pub const fn from_millis(ms: u64) -> Self {
        TimeSpec { seconds: ms / 1_000, nanoseconds: (ms % 1_000) * 1_000_000 }
    }

    pub const fn from_seconds(s: u64) -> Self {
        TimeSpec { seconds: s, nanoseconds: 0 }
    }
}

/// Read the specified clock. Returns `Ok(TimeSpec)` or `Err(errno)`.
pub fn clock_gettime(clock: ClockId) -> Result<TimeSpec, i32> {
    let mut ts = TimeSpec::default();
    let result = raw::raw_clock_gettime(clock as i32, &mut ts as *mut TimeSpec as *mut u64);
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(ts)
    }
}

/// Sleep for the duration described by `duration`. Returns `Ok(())` or `Err(errno)`.
pub fn nanosleep(duration: &TimeSpec) -> Result<(), i32> {
    let result = raw::raw_nanosleep(duration as *const TimeSpec as *const u64);
    if result < 0 {
        Err(result as i32)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DateTimeLocal — timezone-aware datetime
// ---------------------------------------------------------------------------

/// A calendar date and time in a specific timezone.
///
/// Unlike `DateTime` (which is always UTC), `DateTimeLocal` carries the
/// timezone offset and name.  The internal `utc` field is always UTC; the
/// calendar fields (`year`, `month`, ...) are derived from the local time.
///
/// `DateTimeLocal` does NOT implement `Copy` because it contains a `String`
/// (the timezone abbreviation).  Use `.clone()` if you need a second copy.
pub struct DateTimeLocal {
    /// The underlying UTC time.
    pub utc:          DateTime,
    /// Local calendar decomposition (shifted by the UTC offset).
    pub local:        DateTime,
    /// UTC offset in seconds east of UTC.
    pub offset_secs:  i32,
    /// Timezone abbreviation (e.g. `"EST"`, `"EDT"`, `"UTC"`).
    pub tz_abbr:      alloc::string::String,
    /// Whether DST is currently active.
    pub is_dst:       bool,
}

impl DateTimeLocal {
    /// Construct from a UTC `DateTime` and a resolved `Timezone`.
    pub fn from_utc_and_tz(utc: DateTime, tz: &crate::timezone::Timezone) -> DateTimeLocal {
        let unix = utc.to_unix_time_seconds() as i64;
        let offset  = tz.utc_offset_at(unix);
        let is_dst  = tz.is_dst_at(unix);
        let abbr    = alloc::string::String::from(tz.abbreviation_at(unix));
        // Local time = UTC + offset.
        let local_unix = if offset >= 0 {
            utc.to_unix_time_seconds().saturating_add(offset as u64)
        } else {
            utc.to_unix_time_seconds().saturating_sub((-offset) as u64)
        };
        let local = DateTime::from_unix(local_unix, utc.nanosecond);
        DateTimeLocal { utc, local, offset_secs: offset, tz_abbr: abbr, is_dst }
    }

    /// Current local time using `resolve_timezone()`.
    pub fn now() -> DateTimeLocal {
        let utc = DateTime::now();
        let tz  = crate::timezone::resolve_timezone();
        DateTimeLocal::from_utc_and_tz(utc, &tz)
    }

    /// Format the local time using strftime-style directives.
    ///
    /// Extends `DateTime::format` with `%Z` (timezone abbreviation) and
    /// `%z` (UTC offset as `+HHMM`/`-HHMM`).
    pub fn format(&self, fmt: &str) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        let bytes = fmt.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 1 < bytes.len() {
                i += 1;
                match bytes[i] {
                    b'Z' => result.push_str(self.tz_abbr.as_str()),
                    b'z' => {
                        let (sign, abs) = if self.offset_secs >= 0 {
                            ('+', self.offset_secs)
                        } else {
                            ('-', -self.offset_secs)
                        };
                        result.push(sign);
                        dt_push_u8_2(&mut result, (abs / 3600) as u8);
                        dt_push_u8_2(&mut result, ((abs % 3600) / 60) as u8);
                    }
                    // All other specifiers delegate to the local DateTime.
                    spec => {
                        let mut tmp = alloc::string::String::from("%");
                        tmp.push(spec as char);
                        result.push_str(self.local.format(tmp.as_str()).as_str());
                    }
                }
            } else {
                result.push(bytes[i] as char);
            }
            i += 1;
        }
        result
    }

    /// `"Thu Jan  1 00:00:00 EST 1970"` — POSIX `date` format in local time.
    pub fn format_posix_date(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        s.push_str(self.local.weekday_name_short()); s.push(' ');
        s.push_str(self.local.month_name_short());   s.push(' ');
        if self.local.day < 10 { s.push(' '); }
        dt_push_u8_raw(&mut s, self.local.day);      s.push(' ');
        dt_push_u8_2(&mut s, self.local.hour);       s.push(':');
        dt_push_u8_2(&mut s, self.local.minute);     s.push(':');
        dt_push_u8_2(&mut s, self.local.second);     s.push(' ');
        s.push_str(self.tz_abbr.as_str());           s.push(' ');
        dt_push_i32_4(&mut s, self.local.year);
        s
    }
}
