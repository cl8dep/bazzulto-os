#pragma once
// Bazzulto.System — Environment C++ API
//
// Environment variables are deferred (no kernel support in v1.0).
// Compile-time path constants and architecture info are available now.

#include <stdint.h>
#include <stddef.h>

namespace Bazzulto {

struct Environment {
    Environment() = delete;

    // --- Compile-time constants ---

    static constexpr const char* temp_dir()   noexcept { return "/data/temp"; }
    static constexpr const char* home_dir()   noexcept { return "/home/user"; }
    static constexpr const char* os_version() noexcept { return "Bazzulto 1.0"; }
    static constexpr const char* arch()       noexcept { return "aarch64"; }

    // --- Deferred: environment variables (no kernel env-var syscall in v1.0) ---
    // All functions below return nullptr / false / 0 until the kernel
    // implements environment variable support.
    // See docs/tech-debt/bzinit-v1.md.

    /// Returns nullptr — deferred.
    static const char* get(const char* /*key*/) noexcept { return nullptr; }

    /// No-op — deferred.
    static void set(const char* /*key*/, const char* /*value*/) noexcept {}

    /// No-op — deferred.
    static void del(const char* /*key*/) noexcept {}

    // --- Deferred: system info (needs kernel sysinfo syscall) ---

    /// Returns nullptr — deferred.
    static const char* hostname() noexcept { return nullptr; }
    /// Returns nullptr — deferred.
    static const char* username() noexcept { return nullptr; }
    /// Returns 0 — deferred.
    static uint32_t cpu_count() noexcept { return 0; }
    /// Returns 0 — deferred.
    static uint64_t memory_total() noexcept { return 0; }
    /// Returns 0 — deferred.
    static uint64_t memory_available() noexcept { return 0; }
};

} // namespace Bazzulto
