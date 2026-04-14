#pragma once
// Bazzulto.System — signal handling (C++ API)

#include <bazzulto/signal.h>
#include <cstdint>

namespace Bazzulto {

/// Signal handler function type.
using SignalHandler = void(*)(int32_t signal_number);

/// Signal disposition.
enum class SignalDisposition : uint64_t {
    Default = BZ_SIG_DFL,
    Ignore  = BZ_SIG_IGN,
};

/// Install a signal handler. Returns 0 or negative errno.
inline int64_t sigaction(int32_t signal_number, SignalHandler handler,
                         uint64_t* old_handler_out = nullptr) noexcept
{
    return bz_sigaction(signal_number,
                        reinterpret_cast<uint64_t>(handler),
                        old_handler_out);
}

/// Install a special disposition (Default or Ignore). Returns 0 or negative errno.
inline int64_t sigaction(int32_t signal_number, SignalDisposition disposition,
                         uint64_t* old_handler_out = nullptr) noexcept
{
    return bz_sigaction(signal_number,
                        static_cast<uint64_t>(disposition),
                        old_handler_out);
}

/// Send signal to pid. Returns 0 or negative errno.
inline int64_t kill(int32_t pid, int32_t signal_number) noexcept {
    return bz_kill(pid, signal_number);
}

// Signal number constants — brought into the namespace for C++ callers.
inline constexpr int32_t SIGHUP  = BZ_SIGHUP;
inline constexpr int32_t SIGINT  = BZ_SIGINT;
inline constexpr int32_t SIGQUIT = BZ_SIGQUIT;
inline constexpr int32_t SIGILL  = BZ_SIGILL;
inline constexpr int32_t SIGABRT = BZ_SIGABRT;
inline constexpr int32_t SIGFPE  = BZ_SIGFPE;
inline constexpr int32_t SIGKILL = BZ_SIGKILL;
inline constexpr int32_t SIGSEGV = BZ_SIGSEGV;
inline constexpr int32_t SIGPIPE = BZ_SIGPIPE;
inline constexpr int32_t SIGALRM = BZ_SIGALRM;
inline constexpr int32_t SIGTERM = BZ_SIGTERM;
inline constexpr int32_t SIGCHLD = BZ_SIGCHLD;
inline constexpr int32_t SIGCONT = BZ_SIGCONT;
inline constexpr int32_t SIGSTOP = BZ_SIGSTOP;

} // namespace Bazzulto
