#pragma once
// Bazzulto.System — time (C++ API)

#include <bazzulto/time.h>
#include <cstdint>

namespace Bazzulto {

/// Clock identifiers.
enum class ClockId : int32_t {
    Realtime  = BZ_CLOCK_REALTIME,
    Monotonic = BZ_CLOCK_MONOTONIC,
};

/// Typed wrapper around bz_timespec_t — seconds + nanoseconds.
struct TimeSpec {
    uint64_t seconds;
    uint64_t nanoseconds;

    static constexpr TimeSpec from_millis(uint64_t milliseconds) noexcept {
        return { milliseconds / 1'000, (milliseconds % 1'000) * 1'000'000 };
    }

    static constexpr TimeSpec from_seconds(uint64_t seconds) noexcept {
        return { seconds, 0 };
    }

    /// Total nanoseconds (saturates at UINT64_MAX for very large values).
    constexpr uint64_t total_nanoseconds() const noexcept {
        return seconds * 1'000'000'000ULL + nanoseconds;
    }

    /// Convert to the C ABI struct for syscall calls.
    bz_timespec_t to_c() const noexcept {
        return { seconds, nanoseconds };
    }

    static TimeSpec from_c(bz_timespec_t c) noexcept {
        return { c.seconds, c.nanoseconds };
    }
};

/// Read the specified clock. Returns 0 and writes to `out` on success,
/// or returns negative errno on failure.
inline int64_t clock_gettime(ClockId clock, TimeSpec& out) noexcept {
    bz_timespec_t raw{};
    int64_t result = bz_clock_gettime(static_cast<int32_t>(clock), &raw);
    if (result == 0) out = TimeSpec::from_c(raw);
    return result;
}

/// Sleep for `duration`. Returns 0 or negative errno.
inline int64_t nanosleep(const TimeSpec& duration) noexcept {
    bz_timespec_t raw = duration.to_c();
    return bz_nanosleep(&raw);
}

} // namespace Bazzulto
