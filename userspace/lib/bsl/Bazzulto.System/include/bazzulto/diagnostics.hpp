#pragma once
// Bazzulto.System — Diagnostics C++ API
//
// Equivalent to System.Diagnostics in C#.
// Stopwatch is fully implemented via monotonic clock.
// Stack trace and debugger detection are deferred.

#include <stdint.h>

namespace Bazzulto {

// Forward-declare vDSO clock stub.
extern "C" int64_t bz_clock_gettime(int32_t clock_id, uint64_t* ts_out) noexcept;

// ---------------------------------------------------------------------------
// Stopwatch
// ---------------------------------------------------------------------------

/// A monotonic stopwatch. Start it with Stopwatch::start().
struct Stopwatch {
private:
    uint64_t start_nanos_;

    static uint64_t monotonic_nanos() noexcept {
        uint64_t buf[2] = {};
        bz_clock_gettime(1 /* CLOCK_MONOTONIC */, buf);
        return buf[0] * 1'000'000'000ULL + buf[1];
    }

public:
    static Stopwatch start() noexcept {
        Stopwatch sw;
        sw.start_nanos_ = monotonic_nanos();
        return sw;
    }

    /// Elapsed nanoseconds since start().
    uint64_t elapsed_ns() const noexcept {
        uint64_t now = monotonic_nanos();
        return now > start_nanos_ ? now - start_nanos_ : 0;
    }

    /// Elapsed milliseconds since start().
    uint64_t elapsed_ms() const noexcept { return elapsed_ns() / 1'000'000ULL; }

    /// Elapsed seconds since start().
    uint64_t elapsed_secs() const noexcept { return elapsed_ns() / 1'000'000'000ULL; }

    /// Reset the stopwatch to the current time.
    void reset() noexcept { start_nanos_ = monotonic_nanos(); }
};

// ---------------------------------------------------------------------------
// ProcessInfo — per-process diagnostic snapshot
// ---------------------------------------------------------------------------

struct ProcessInfo {
    int32_t pid;
    // cpu_usage, memory_usage, threads, open_files, start_time — all deferred.
    // Returns 0 / empty until kernel exposes a sysinfo syscall.

    uint32_t cpu_usage_percent() const noexcept { return 0; }  ///< Deferred.
    uint64_t memory_usage_bytes() const noexcept { return 0; } ///< Deferred.

    static ProcessInfo current() noexcept;  ///< Implemented via bz_getpid().
    /// Returns a ProcessInfo with pid = -1 if not found. Deferred lookup.
    static ProcessInfo find_by_pid(int32_t pid) noexcept { return { pid }; }
    /// Deferred — returns ProcessInfo with pid = -1.
    static ProcessInfo find_by_name(const char* /*name*/) noexcept { return { -1 }; }
};

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

struct Diagnostics {
    Diagnostics() = delete;

    /// Always false in v1.0 — deferred.
    static bool is_debugger_attached() noexcept { return false; }

    /// No-op in v1.0 — deferred.
    static void break_if_debugging() noexcept {}

    /// Writes msg to stderr and triggers a fault if condition is false.
    static void assert_that(bool condition, const char* msg) noexcept;

    /// No-op in v1.0 — deferred (no unwind info available).
    static void print_stack_trace() noexcept {}
};

} // namespace Bazzulto
