#pragma once
// Bazzulto.System — Info C++ API
//
// Equivalent to System.Runtime in C#.
// OS/arch constants are compile-time. PID/PPID come from syscalls.
// Hardware info (CPU brand, memory, uptime) is deferred (needs sysinfo syscall).

#include <stdint.h>

// Forward-declare the vDSO call stubs used for getpid/getppid.
extern "C" int64_t bz_getpid()  noexcept;
extern "C" int64_t bz_getppid() noexcept;

namespace Bazzulto {

struct Info {
    Info() = delete;

    // --- Compile-time constants ---

    static constexpr const char* os_name()        noexcept { return "Bazzulto"; }
    static constexpr const char* os_version()     noexcept { return "1.0.0"; }
    static constexpr const char* kernel_version() noexcept { return "0.1.0-dev"; }
    static constexpr const char* arch()           noexcept { return "aarch64"; }

    // --- Syscall-backed ---

    static int32_t pid()  noexcept { return static_cast<int32_t>(bz_getpid());  }
    static int32_t ppid() noexcept { return static_cast<int32_t>(bz_getppid()); }

    // --- Deferred: hardware info (needs kernel sysinfo syscall) ---
    // See docs/tech-debt/bzinit-v1.md.

    /// Returns nullptr — deferred.
    static const char* cpu_brand()   noexcept { return nullptr; }
    /// Returns 0 — deferred.
    static uint32_t    cpu_count()   noexcept { return 0; }
    /// Returns 0 — deferred.
    static uint64_t    memory_total() noexcept { return 0; }
    /// Returns nullptr — deferred.
    static const char* executable()  noexcept { return nullptr; }
};

} // namespace Bazzulto
