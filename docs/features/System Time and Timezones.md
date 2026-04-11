# System Time and Timezones

**Priority:** Medium — implement after VFS is stable.
**Dependencies:**
- `docs/features/System files and folder structure.md` — requires `//system:/etc/timezone`
  and `//system:/etc/zoneinfo/` to be readable via the VFS for full timezone support.
- RTC hardware (PL031 at `0x09010000` on QEMU `virt`) for wall-clock time.
- Architected timer (`CNTVCT_EL0`) already present for monotonic time.

**Area:** `kernel/drivers/rtc.c`, `userspace/libc/time.h`, `userspace/libc/time.c`

---

## Overview

System time in Bazzulto is split into three independent concerns:

| Concern | Source | Timezone needed |
|---|---|---|
| Monotonic time | `CNTVCT_EL0` — never jumps | No |
| Wall-clock UTC | PL031 RTC | No |
| Local time | UTC + timezone rules | Yes |

These are exposed through a `time.h` libc API modelled on C11 §7.27, extended
with a Bazzulto-specific monotonic clock.

---

## Hardware Layer

### PL031 RTC (wall clock)

The QEMU `virt` machine exposes a PL011-compatible RTC at `0x09010000`.
Register `RTCDR` (offset `0x000`) is a 32-bit read-only counter of seconds
elapsed since the Unix epoch (1970-01-01 00:00:00 UTC).

```c
// kernel/drivers/rtc.c

#define PL031_BASE       0x09010000UL
#define PL031_RTCDR      (*(volatile uint32_t *)(PL031_BASE + 0x000))

// Returns seconds since Unix epoch, UTC.
uint32_t rtc_read_seconds(void) {
    return PL031_RTCDR;
}
```

Reference: PL031 TRM (ARM DDI 0224).

### Architected timer (monotonic clock)

`CNTVCT_EL0` increments at a fixed frequency given by `CNTFRQ_EL0`. This
counter never jumps backwards and is unaffected by timezone or NTP adjustments.
Use it for measuring elapsed time, timeouts, and scheduling — not for
displaying wall-clock time.

```c
static inline uint64_t monotonic_ticks(void) {
    uint64_t count;
    __asm__ volatile("mrs %0, cntvct_el0" : "=r"(count));
    return count;
}

static inline uint64_t timer_frequency_hz(void) {
    uint64_t freq;
    __asm__ volatile("mrs %0, cntfrq_el0" : "=r"(freq));
    return freq;
}

// Returns nanoseconds since boot.
uint64_t monotonic_nanoseconds(void) {
    return monotonic_ticks() * 1000000000ULL / timer_frequency_hz();
}
```

---

## Kernel Syscalls

Two syscalls expose time to userspace:

```c
/// Return seconds since Unix epoch (UTC), read from the PL031 RTC.
/// Returns the time_t value. Never fails.
#define SYS_TIME  17

/// Return nanoseconds since boot from the architected timer.
/// Returns a uint64_t. Never fails.
#define SYS_CLOCK  18
```

---

## libc API (`userspace/libc/time.h`)

### Types

```c
typedef uint32_t time_t;      // seconds since Unix epoch (UTC), 32-bit until 2038
typedef uint64_t clock_t;     // nanoseconds since boot (monotonic)

struct tm {
    int tm_sec;    // seconds       0–60  (60 for leap second)
    int tm_min;    // minutes       0–59
    int tm_hour;   // hours         0–23
    int tm_mday;   // day of month  1–31
    int tm_mon;    // month         0–11  (0 = January)
    int tm_year;   // years since 1900
    int tm_wday;   // weekday       0–6   (0 = Sunday)
    int tm_yday;   // day of year   0–365
    int tm_isdst;  // DST in effect: 1=yes, 0=no, -1=unknown
};
```

### Functions

```c
// Return current UTC time as seconds since epoch.
time_t time(time_t *out);

// Return nanoseconds since boot (monotonic, never jumps).
clock_t clock(void);

// Convert time_t to broken-down UTC time (no timezone applied).
struct tm gmtime(time_t t);

// Convert time_t to broken-down local time (applies configured timezone).
struct tm localtime(time_t t);

// Convert broken-down time back to time_t (treats input as UTC).
time_t mktime(struct tm *tm);

// Format a time value into a string.
// Supported directives: %Y %m %d %H %M %S %A %a %B %b %Z %z %%
// Returns number of bytes written (not counting null terminator).
size_t strftime(char *buf, size_t size, const char *format, const struct tm *tm);

// Compute difference in seconds between two time_t values.
double difftime(time_t end, time_t start);
```

---

## Timezone Implementation

### Phase 1 — Fixed UTC offset (no DST)

The first implementation reads a single offset from
`//system:/etc/timezone.offset` — a plain-text file containing a signed integer
in seconds:

```
// //system:/etc/timezone.offset
-18000
```

`localtime()` applies the offset directly:

```c
struct tm localtime(time_t t) {
    int32_t offset = read_timezone_offset();  // reads //system:/etc/timezone.offset
    return gmtime((time_t)((int64_t)t + offset));
}
```

This covers all timezones without DST (UTC-12 through UTC+14). Sufficient for
initial bring-up.

### Phase 2 — Full IANA tzdata (with DST)

Full timezone support requires the IANA timezone database stored under
`//system:/etc/zoneinfo/`. This is a dependency on the VFS being functional.

**File layout:**

```
//system:/etc/timezone                   → plain text: "Europe/Madrid"
//system:/etc/zoneinfo/
    Europe/
        Madrid                           → TZif v2 binary file
        London
    America/
        New_York
        Bogota
    UTC
    ...
```

**TZif binary format (v2/v3):**

Each file contains a list of transition timestamps and the UTC offsets and DST
flags that apply after each transition. The kernel does not parse these —
`localtime()` in userspace reads the file via `open`/`read` and applies the
correct rule for the given `time_t`.

```c
struct tm localtime(time_t t) {
    // 1. Read //system:/etc/timezone → zone name (e.g. "Europe/Madrid")
    // 2. Open //system:/etc/zoneinfo/Europe/Madrid
    // 3. Parse TZif header and transition table
    // 4. Binary-search the transition table for t
    // 5. Apply the matching UTC offset and DST flag
    // 6. Call gmtime(t + offset)
}
```

The TZif parser is approximately 300–400 lines of C. The database files are
sourced from the IANA tz distribution (`data.iana.org/time-zones`) and embedded
in the system image at build time.

---

## `strftime` Format Directives

| Directive | Output | Example |
|---|---|---|
| `%Y` | 4-digit year | `2026` |
| `%m` | Month (01–12) | `04` |
| `%d` | Day (01–31) | `11` |
| `%H` | Hour 24h (00–23) | `14` |
| `%M` | Minute (00–59) | `05` |
| `%S` | Second (00–60) | `09` |
| `%A` | Full weekday name | `Saturday` |
| `%a` | Abbreviated weekday | `Sat` |
| `%B` | Full month name | `April` |
| `%b` | Abbreviated month | `Apr` |
| `%Z` | Timezone name | `CET`, `UTC` |
| `%z` | UTC offset | `+0100` |
| `%%` | Literal `%` | `%` |

---

## Code Examples

### Get current UTC time

```c
#include <libc/time.h>
#include <libc/stdio.h>

void print_utc(void) {
    time_t now = time(NULL);
    struct tm t = gmtime(now);
    char buf[32];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S UTC", &t);
    printf("%s\n", buf);
    // → "2026-04-11 14:05:09 UTC"
}
```

### Get local time

```c
void print_local(void) {
    time_t now = time(NULL);
    struct tm t = localtime(now);
    char buf[48];
    strftime(buf, sizeof(buf), "%A %d %B %Y %H:%M:%S %Z", &t);
    printf("%s\n", buf);
    // → "Saturday 11 April 2026 16:05:09 CEST"
}
```

### Measure elapsed time with monotonic clock

```c
void benchmark(void) {
    clock_t start = clock();

    // ... work ...

    clock_t end = clock();
    uint64_t elapsed_ms = (end - start) / 1000000ULL;
    printf("elapsed: %llu ms\n", elapsed_ms);
}
```

### Compute time difference

```c
void days_until(int year, int month, int day) {
    time_t now = time(NULL);

    struct tm target = {
        .tm_year = year - 1900,
        .tm_mon  = month - 1,
        .tm_mday = day,
    };
    time_t target_t = mktime(&target);

    double diff = difftime(target_t, now);
    printf("%d days remaining\n", (int)(diff / 86400));
}
```

---

## Implementation Order

1. **`rtc_read_seconds()`** in `kernel/drivers/rtc.c` — read PL031, expose via `SYS_TIME`
2. **`SYS_CLOCK`** — expose `CNTVCT_EL0` via syscall
3. **`gmtime()`** — arithmetic only, no I/O
4. **`mktime()`** — inverse of gmtime
5. **`strftime()`** — string formatting
6. **`localtime()` Phase 1** — fixed offset from `//system:/etc/timezone.offset`
7. **`time.h` tests** — host-compiled, same pattern as existing libc tests
8. **`localtime()` Phase 2** — TZif parser, requires VFS

---

## Future Optimization — vDSO

Currently `SYS_TIME` and `SYS_CLOCK` are real syscalls. Every call crosses the
user/kernel boundary (~200–500 ns per call). For most workloads this is
acceptable.

Linux solves this with **vDSO** (virtual Dynamic Shared Object): the kernel
maps a small read-only page into every process's address space containing a
copy of the current time value and the code to read it. `clock_gettime()` in
libc detects the vDSO and reads directly from that page — no syscall, no
context switch, ~10 ns per call.

```
// Without vDSO (Bazzulto today):
time()  →  SYS_TIME  →  kernel reads PL031  →  returns  (~300 ns)

// With vDSO (future):
time()  →  read vDSO page  →  returns  (~10 ns)
             ↑
             kernel updates this page on every timer tick
```

**What the kernel must maintain for vDSO:**

```c
// Shared page layout — written by kernel, read by userspace
typedef struct {
    uint32_t wall_clock_seconds;   // updated by kernel on every RTC tick
    uint64_t monotonic_ticks;      // updated on every timer interrupt
    uint64_t timer_frequency_hz;   // set once at boot
    uint32_t sequence_lock;        // reader checks before and after to detect torn reads
} vdso_data_t;
```

The sequence lock prevents a process from reading a partially-updated value if
the kernel is updating the page at the same moment.

**Prerequisites for vDSO in Bazzulto:**

1. A mapped shared page per process (requires `mmap` and the ELF loader to
   inject the mapping at process creation)
2. Kernel updates the page on every timer interrupt
3. libc `time()` and `clock()` check for the vDSO page before falling back to
   the syscall

This is a performance optimization only. The syscall path remains correct and
is used as fallback. Implement after the scheduler and timer are fully stable.

---

## Known Limitations

- `time_t` is 32-bit and will overflow on 2038-01-19 (the Year 2038 problem).
  A future version should use `int64_t`. This is tracked in `TECHNICAL_DEBT.md`.
- Phase 1 does not handle DST. Apps that need DST correctness must wait for
  Phase 2.
- The RTC on real hardware may drift. NTP synchronization is a future feature
  and is not part of this document.

---

## References

- PL031 RTC TRM: ARM DDI 0224
- ARM Architected Timer: ARM DDI 0487 D1.8 (Generic Timer)
- IANA timezone database: `data.iana.org/time-zones`
- TZif file format: RFC 8536
- C11 time functions: ISO/IEC 9899:2011 §7.27
