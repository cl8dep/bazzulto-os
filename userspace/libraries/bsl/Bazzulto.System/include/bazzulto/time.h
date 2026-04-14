#pragma once
/**
 * @file time.h
 * @brief Bazzulto.System — time and calendar C ABI.
 *
 * Provides:
 *   - Clock read access via bz_clock_gettime() / bz_nanosleep().
 *   - Calendar decomposition via bz_datetime_t and its companion functions,
 *     analogous to C#'s System.DateTime.
 *
 * All times are UTC.  There is no timezone conversion support in v1.
 *
 * @note bz_clock_gettime(BZ_CLOCK_MONOTONIC/REALTIME) executes entirely in
 *       userspace via the vDSO fast path (no kernel trap for those two clock
 *       IDs).  All other syscalls in this header do trap into the kernel.
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------
 * Clock identifiers
 * ------------------------------------------------------------------------- */

/** @defgroup clock_ids Clock Identifiers
 *  @{
 */
/** Wall-clock time (Unix epoch).  May jump on NTP adjustments. */
#define BZ_CLOCK_REALTIME   0
/** Monotonically increasing clock since boot.  Never jumps backward. */
#define BZ_CLOCK_MONOTONIC  1
/** @} */

/* ---------------------------------------------------------------------------
 * bz_timespec_t — POSIX-compatible time value
 * ------------------------------------------------------------------------- */

/**
 * @brief A point in time with nanosecond resolution, analogous to
 *        struct timespec in POSIX.
 */
typedef struct {
    uint64_t seconds;      /**< Whole seconds since the clock epoch. */
    uint64_t nanoseconds;  /**< Sub-second part, 0–999 999 999. */
} bz_timespec_t;

/* ---------------------------------------------------------------------------
 * bz_datetime_t — calendar decomposition, analogous to C# System.DateTime
 * ------------------------------------------------------------------------- */

/**
 * @brief Gregorian calendar date and time in UTC.
 *
 * Analogous to C#'s System.DateTime with Kind = DateTimeKind.Utc.
 * Populate with bz_datetime_now(), bz_datetime_from_unix(), or
 * bz_datetime_parse().  Arithmetic is done with the bz_datetime_add_*()
 * family.
 *
 * Fields are read-only after construction — pass the struct by value and
 * use the helper functions to produce modified copies.
 */
typedef struct {
    int32_t  year;        /**< Year (e.g. 2025). */
    uint8_t  month;       /**< Month 1–12. */
    uint8_t  day;         /**< Day of month 1–31. */
    uint8_t  hour;        /**< Hour 0–23. */
    uint8_t  minute;      /**< Minute 0–59. */
    uint8_t  second;      /**< Second 0–59. */
    uint8_t  weekday;     /**< Day of week: 0 = Sunday, …, 6 = Saturday. */
    uint8_t  _pad[2];     /**< Reserved — must be zero. */
    uint32_t nanosecond;  /**< Nanosecond 0–999 999 999. */
    uint64_t unix_seconds;/**< Cached Unix timestamp (private — do not modify). */
} bz_datetime_t;

/**
 * @brief bz_rusage_t — resource usage statistics.
 *
 * Subset of POSIX struct rusage returned by bz_getrusage().
 * Only user and system CPU time are currently meaningful;
 * all other fields are zero (stub).
 */
typedef struct {
    bz_timespec_t user_time;    /**< User-space CPU time consumed. */
    bz_timespec_t system_time;  /**< Kernel CPU time consumed (stub: always 0). */
    uint64_t      _reserved[14];/**< Padding for future POSIX fields. */
} bz_rusage_t;

/* ---------------------------------------------------------------------------
 * Clock functions
 * ------------------------------------------------------------------------- */

/**
 * @brief Read the current value of a clock.
 *
 * For BZ_CLOCK_REALTIME and BZ_CLOCK_MONOTONIC this executes entirely in
 * userspace via the vDSO — no kernel trap occurs.
 *
 * @param clock_id  Clock to read (BZ_CLOCK_REALTIME or BZ_CLOCK_MONOTONIC).
 * @param ts_out    Written with the current time on success.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_clock_gettime(int32_t clock_id, bz_timespec_t *ts_out);

/**
 * @brief Suspend the calling process for at least the specified duration.
 *
 * @param req  Duration to sleep.  Both fields must be normalised
 *             (nanoseconds < 1 000 000 000).
 * @return 0 on success, -BZ_EINTR if interrupted by a signal, or another
 *         negative errno value.
 */
int64_t bz_nanosleep(const bz_timespec_t *req);

/* ---------------------------------------------------------------------------
 * DateTime construction
 * ------------------------------------------------------------------------- */

/**
 * @brief Read the current UTC wall-clock time and decompose it.
 *
 * Equivalent to C# DateTime.UtcNow.
 *
 * @param out  Written with the current date and time.
 */
void bz_datetime_now(bz_datetime_t *out);

/**
 * @brief Decompose a Unix timestamp into UTC calendar fields.
 *
 * Equivalent to DateTimeOffset.FromUnixTimeSeconds(unix_seconds).UtcDateTime.
 *
 * @param unix_seconds  Seconds since 1970-01-01 00:00:00 UTC.
 * @param nanosecond    Sub-second nanoseconds 0–999 999 999.
 * @param out           Written with the decomposed date and time.
 */
void bz_datetime_from_unix(uint64_t unix_seconds, uint32_t nanosecond,
                            bz_datetime_t *out);

/**
 * @brief Construct a DateTime from individual UTC components.
 *
 * Equivalent to new DateTime(year, month, day, hour, minute, second,
 *                            DateTimeKind.Utc) in C#.
 *
 * @param year    Full year (e.g. 2025).
 * @param month   Month 1–12.
 * @param day     Day of month 1–31.
 * @param hour    Hour 0–23.
 * @param minute  Minute 0–59.
 * @param second  Second 0–59.
 * @param out     Written on success.
 * @return 0 on success, -1 if any field is out of range.
 */
int bz_datetime_new(int32_t year, uint8_t month, uint8_t day,
                    uint8_t hour, uint8_t minute, uint8_t second,
                    bz_datetime_t *out);

/**
 * @brief Return midnight of the current UTC day.
 *
 * Equivalent to DateTime.Today in C#.
 *
 * @param out  Written with today's date at 00:00:00.000000000 UTC.
 */
void bz_datetime_today(bz_datetime_t *out);

/**
 * @brief Parse a date/time string in "YYYY-MM-DD" or "YYYY-MM-DD HH:MM:SS"
 *        format (UTC assumed).
 *
 * Equivalent to DateTime.Parse(s) in C#.
 *
 * @param s    Null-terminated input string.
 * @param out  Written on successful parse.
 * @return 0 on success, -1 on malformed input or out-of-range fields.
 */
int bz_datetime_parse(const char *s, bz_datetime_t *out);

/* ---------------------------------------------------------------------------
 * DateTime properties
 * ------------------------------------------------------------------------- */

/**
 * @brief Day of the year, 1-based (Jan 1 = 1, Dec 31 = 365 or 366).
 *
 * Equivalent to DateTime.DayOfYear in C#.
 */
uint16_t bz_datetime_day_of_year(const bz_datetime_t *dt);

/**
 * @brief Millisecond component (0–999), derived from nanosecond.
 *
 * Equivalent to DateTime.Millisecond in C#.
 */
uint16_t bz_datetime_millisecond(const bz_datetime_t *dt);

/**
 * @brief Unix timestamp in seconds (seconds since 1970-01-01 00:00:00 UTC).
 *
 * Equivalent to DateTimeOffset.ToUnixTimeSeconds() in C#.
 */
uint64_t bz_datetime_to_unix_seconds(const bz_datetime_t *dt);

/**
 * @brief Unix timestamp in milliseconds.
 *
 * Equivalent to DateTimeOffset.ToUnixTimeMilliseconds() in C#.
 */
uint64_t bz_datetime_to_unix_milliseconds(const bz_datetime_t *dt);

/**
 * @brief 100-nanosecond ticks since 0001-01-01 00:00:00 UTC.
 *
 * Equivalent to DateTime.Ticks in C#.
 */
uint64_t bz_datetime_ticks(const bz_datetime_t *dt);

/* ---------------------------------------------------------------------------
 * Calendar helpers (static / pure functions)
 * ------------------------------------------------------------------------- */

/**
 * @brief Return non-zero if @p year is a Gregorian leap year.
 *
 * Equivalent to DateTime.IsLeapYear(year) in C#.
 */
int bz_datetime_is_leap_year(int32_t year);

/**
 * @brief Number of days in the given month of the given year (1–28/29/30/31).
 *
 * Equivalent to DateTime.DaysInMonth(year, month) in C#.
 *
 * @return Days in month, or 0 if @p month is out of range.
 */
uint8_t bz_datetime_days_in_month(int32_t year, uint8_t month);

/* ---------------------------------------------------------------------------
 * DateTime arithmetic
 * ------------------------------------------------------------------------- */

/** @brief Add whole seconds (negative subtracts).  Returns modified copy. */
bz_datetime_t bz_datetime_add_seconds(bz_datetime_t dt, int64_t seconds);
/** @brief Add whole minutes. */
bz_datetime_t bz_datetime_add_minutes(bz_datetime_t dt, int64_t minutes);
/** @brief Add whole hours. */
bz_datetime_t bz_datetime_add_hours(bz_datetime_t dt, int64_t hours);
/** @brief Add whole days. */
bz_datetime_t bz_datetime_add_days(bz_datetime_t dt, int64_t days);
/** @brief Add whole weeks. */
bz_datetime_t bz_datetime_add_weeks(bz_datetime_t dt, int64_t weeks);

/**
 * @brief Add calendar months, clamping the day if the target month is shorter.
 *
 * Equivalent to DateTime.AddMonths(n) in C#.
 */
bz_datetime_t bz_datetime_add_months(bz_datetime_t dt, int32_t months);

/**
 * @brief Add calendar years.  Feb 29 is clamped to Feb 28 on non-leap targets.
 *
 * Equivalent to DateTime.AddYears(n) in C#.
 */
bz_datetime_t bz_datetime_add_years(bz_datetime_t dt, int32_t years);

/**
 * @brief Compute the elapsed seconds between two DateTime values (lhs - rhs).
 *
 * Returns 0 if @p rhs is after @p lhs.
 * For sub-second precision use the nanosecond fields directly.
 *
 * Equivalent to (lhs - rhs).TotalSeconds in C#.
 */
int64_t bz_datetime_diff_seconds(bz_datetime_t lhs, bz_datetime_t rhs);

/* ---------------------------------------------------------------------------
 * DateTime comparison
 * ------------------------------------------------------------------------- */

/**
 * @brief Compare two DateTime values.
 *
 * @return Negative if @p a < @p b, 0 if equal, positive if @p a > @p b.
 * Equivalent to DateTime.CompareTo(other) in C#.
 */
int bz_datetime_compare(bz_datetime_t a, bz_datetime_t b);

/* ---------------------------------------------------------------------------
 * DateTime formatting
 * ------------------------------------------------------------------------- */

/**
 * @brief Write "YYYY-MM-DD HH:MM:SS" (ISO 8601 UTC) into @p buf.
 *
 * @p buf must be at least 20 bytes (19 chars + null terminator).
 *
 * Equivalent to DateTime.ToString("yyyy-MM-dd HH:mm:ss") in C#.
 */
void bz_datetime_format_iso8601(const bz_datetime_t *dt, char *buf, size_t buf_len);

/**
 * @brief Write "YYYY-MM-DDTHH:MM:SSZ" (RFC 3339) into @p buf.
 *
 * @p buf must be at least 21 bytes.
 */
void bz_datetime_format_rfc3339(const bz_datetime_t *dt, char *buf, size_t buf_len);

/**
 * @brief Write "Thu Jan  1 00:00:00 UTC 1970" (POSIX date format) into @p buf.
 *
 * @p buf must be at least 30 bytes.
 */
void bz_datetime_format_posix(const bz_datetime_t *dt, char *buf, size_t buf_len);

/**
 * @brief Apply a strftime-style format string.
 *
 * Supported directives: %Y %y %m %d %H %M %S %f %j %A %a %B %b %Z %z %n %t %%
 *
 * @param dt       Source date/time.
 * @param fmt      Null-terminated format string.
 * @param buf      Output buffer.
 * @param buf_len  Size of @p buf in bytes (output is null-terminated and
 *                 truncated if necessary).
 */
void bz_datetime_format(const bz_datetime_t *dt, const char *fmt,
                        char *buf, size_t buf_len);

/**
 * @brief Return the abbreviated weekday name ("Sun", "Mon", …, "Sat").
 *
 * @return Pointer to a string literal — do not free.
 */
const char *bz_datetime_weekday_short(const bz_datetime_t *dt);

/**
 * @brief Return the full weekday name ("Sunday", "Monday", …, "Saturday").
 *
 * @return Pointer to a string literal — do not free.
 */
const char *bz_datetime_weekday_long(const bz_datetime_t *dt);

/**
 * @brief Return the abbreviated month name ("Jan", "Feb", …, "Dec").
 *
 * @return Pointer to a string literal — do not free.
 */
const char *bz_datetime_month_short(const bz_datetime_t *dt);

/**
 * @brief Return the full month name ("January", "February", …, "December").
 *
 * @return Pointer to a string literal — do not free.
 */
const char *bz_datetime_month_long(const bz_datetime_t *dt);

/* ---------------------------------------------------------------------------
 * Resource usage
 * ------------------------------------------------------------------------- */

/** who argument for bz_getrusage(). */
#define BZ_RUSAGE_SELF      0   /**< Resource usage of the calling process. */
#define BZ_RUSAGE_CHILDREN  (-1) /**< Resource usage of waited-for children. */

/**
 * @brief Return resource usage statistics.
 *
 * Only user_time is currently populated; system_time is always zero (stub).
 *
 * @param who   BZ_RUSAGE_SELF or BZ_RUSAGE_CHILDREN.
 * @param usage Written with resource usage on success.
 * @return 0 on success, or a negative errno value.
 */
int64_t bz_getrusage(int32_t who, bz_rusage_t *usage);

#ifdef __cplusplus
} /* extern "C" */
#endif
