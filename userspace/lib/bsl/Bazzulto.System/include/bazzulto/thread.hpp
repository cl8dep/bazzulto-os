#pragma once
// Bazzulto.System — Thread C++ API
//
// Thread::sleep() and Thread::yield_now() are implemented via syscalls.
// Thread::spawn() and ThreadPool are deferred (no kernel thread support in v1.0).

#include <stdint.h>

namespace Bazzulto {

// ---------------------------------------------------------------------------
// Duration (minimal, for Thread::sleep)
// ---------------------------------------------------------------------------

/// A span of time in nanoseconds.
struct Duration {
    uint64_t nanos;

    static constexpr Duration ms(uint64_t millis) noexcept {
        return { millis * 1'000'000ULL };
    }
    static constexpr Duration secs(uint64_t seconds) noexcept {
        return { seconds * 1'000'000'000ULL };
    }
    static constexpr Duration mins(uint64_t minutes) noexcept {
        return { minutes * 60ULL * 1'000'000'000ULL };
    }

    constexpr uint64_t as_secs()  const noexcept { return nanos / 1'000'000'000ULL; }
    constexpr uint64_t as_millis() const noexcept { return nanos / 1'000'000ULL; }
};

// ---------------------------------------------------------------------------
// ThreadId
// ---------------------------------------------------------------------------

struct ThreadId {
    int32_t value;  ///< Same as PID in v1.0 (no real threads yet).
};

// ---------------------------------------------------------------------------
// CurrentThread
// ---------------------------------------------------------------------------

struct CurrentThread {
    ThreadId id() const noexcept;
    /// Always returns "main" in v1.0.
    const char* name() const noexcept { return "main"; }
    /// Deferred — no thread naming support in v1.0.
    void set_name(const char* /*name*/) noexcept {}
};

// ---------------------------------------------------------------------------
// ThreadHandle — deferred
// ---------------------------------------------------------------------------

struct ThreadHandle {
    /// Deferred — always returns -38 (ENOSYS).
    int64_t join() noexcept { return -38; }
};

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

struct Thread {
    Thread() = delete;

    static CurrentThread current() noexcept { return {}; }

    /// Sleep for `duration`. Uses the nanosleep syscall.
    static int64_t sleep(Duration duration) noexcept;

    /// Yield the CPU to the scheduler.
    static void yield_now() noexcept;

    /// Deferred — returns -38 (ENOSYS). No kernel thread support in v1.0.
    /// See docs/tech-debt/bzinit-v1.md.
    template <typename F>
    static int64_t spawn(F&&) noexcept { return -38; }
};

// ---------------------------------------------------------------------------
// ThreadPool — deferred
// ---------------------------------------------------------------------------

/// All methods return -38 (ENOSYS) in v1.0.
struct ThreadPool {
    explicit ThreadPool(uint32_t workers) noexcept : workers_(workers) {}

    template <typename F>
    int64_t execute(F&&) noexcept { return -38; }

    uint32_t worker_count() const noexcept { return workers_; }

private:
    uint32_t workers_;
};

} // namespace Bazzulto
