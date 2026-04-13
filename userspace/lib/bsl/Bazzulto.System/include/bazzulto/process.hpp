#pragma once
// Bazzulto.System — process management (C++ API)

#include <bazzulto/process.h>
#include <cstdint>

namespace Bazzulto {

/// Result of a fork() call — discriminated union.
struct ForkResult {
    bool is_child;
    int32_t pid;   // 0 in child, child PID in parent

    static ForkResult from_raw(int64_t raw) noexcept {
        if (raw == 0) return { true, 0 };
        return { false, static_cast<int32_t>(raw) };
    }
};

/// Terminate the current process. Never returns.
[[noreturn]] inline void exit(int32_t code) noexcept {
    bz_exit(code);
}

/// Fork. Returns ForkResult or a negative errno on failure.
inline int64_t fork(ForkResult& result) noexcept {
    int64_t raw = bz_fork();
    if (raw < 0) return raw;
    result = ForkResult::from_raw(raw);
    return 0;
}

/// Replace current image with the binary at path. Only returns on error.
inline int64_t exec(const char* path, size_t path_len) noexcept {
    return bz_exec(path, path_len);
}

/// Spawn a child from a ramfs path. Returns child PID or negative errno.
inline int64_t spawn(const char* path, size_t path_len) noexcept {
    return bz_spawn(path, path_len);
}

/// Wait for a child (pid = -1 for any). Returns child PID or negative errno.
inline int64_t wait(int32_t pid, int32_t* status_out = nullptr) noexcept {
    return bz_wait(pid, status_out);
}

/// Return current process PID.
inline int32_t getpid() noexcept {
    return static_cast<int32_t>(bz_getpid());
}

/// Return parent process PID.
inline int32_t getppid() noexcept {
    return static_cast<int32_t>(bz_getppid());
}

/// Send a signal to a process. Returns 0 or negative errno.
inline int64_t kill(int32_t pid, int32_t signal_number) noexcept {
    return bz_kill(pid, signal_number);
}

} // namespace Bazzulto
