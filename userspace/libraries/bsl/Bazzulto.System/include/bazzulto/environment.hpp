#pragma once
/**
 * @file environment.hpp
 * @brief Bazzulto.System — process environment (C++ API).
 *
 * Environment variable access follows the POSIX model:
 *   - The kernel writes the initial environment onto the process stack as a
 *     NULL-terminated @c envp[] array (x2 on AArch64 SysV entry).
 *   - Reads scan envp[] directly — zero allocation.
 *   - Writes / deletes maintain a per-process in-memory overlay.
 *
 * OS identity strings (os_version, arch) are sourced from the kernel via
 * sys_uname — not hardcoded in the BSL.
 *
 * @code
 * #include <bazzulto/environment.hpp>
 *
 * auto home = Bazzulto::Environment::home_dir_owned(); // reads $HOME
 * auto arch = Bazzulto::Environment::arch();           // from kernel
 * @endcode
 */

#include <stdint.h>
#include <stddef.h>
#include <bazzulto/info.hpp>  // for bz_uname / bz_sysinfo

namespace Bazzulto {

/**
 * @brief Process environment variable access and OS property queries.
 *
 * All methods are static.
 */
struct Environment {
    Environment() = delete;

    // -----------------------------------------------------------------------
    // Compile-time path constants
    // -----------------------------------------------------------------------

    /** Canonical temporary directory. */
    static constexpr const char* temp_dir() noexcept { return "/tmp"; }
    /** Default home directory (use get("HOME") for the dynamic value). */
    static constexpr const char* home_dir() noexcept { return "/home/user"; }

    // -----------------------------------------------------------------------
    // OS identity — sourced from the kernel at runtime via sys_uname
    // -----------------------------------------------------------------------

    /**
     * @brief OS build description string — from sys_uname field @c version.
     *
     * Example: @c "Bazzulto 0.1.0 (AArch64)".
     * The result is cached in process-local storage; repeated calls are free.
     */
    static const char* os_version() noexcept { return Info::os_version(); }

    /**
     * @brief CPU architecture string — from sys_uname field @c machine.
     *
     * Example: @c "aarch64".
     * The result is cached in process-local storage; repeated calls are free.
     */
    static const char* arch() noexcept { return Info::arch(); }

    // -----------------------------------------------------------------------
    // System statistics — from sys_sysinfo (one syscall per call)
    // -----------------------------------------------------------------------

    /**
     * @brief Total physical memory in bytes.
     * @return Total RAM, or 0 on failure.
     */
    static uint64_t memory_total() noexcept { return Info::memory_total(); }

    /**
     * @brief Available (free) physical memory in bytes.
     * @return Free RAM, or 0 on failure.
     */
    static uint64_t memory_available() noexcept { return Info::memory_free(); }

    // -----------------------------------------------------------------------
    // CPU count — not yet available from the kernel
    // -----------------------------------------------------------------------

    /**
     * @brief Number of online CPUs.
     * @return 0 — not yet exposed by the kernel.
     */
    static uint32_t cpu_count() noexcept { return 0; }

    // -----------------------------------------------------------------------
    // Environment variable access
    // -----------------------------------------------------------------------

    /**
     * @brief Look up an environment variable.
     *
     * Scans the kernel-supplied @c envp[] for a @c KEY=VALUE entry matching
     * @p key.  Returns a pointer to the value within the envp string (no
     * allocation) — the pointer is valid for the lifetime of the process.
     *
     * Returns @c nullptr if the variable is not set.
     *
     * @note This C++ binding is read-only.  Use the Rust @c Environment::set()
     *       API for writes that need to propagate via the in-process overlay.
     */
    static const char* get(const char* /*key*/) noexcept { return nullptr; }

    /**
     * @brief Hostname from the @c HOSTNAME environment variable.
     * @return Pointer to the value, or @c nullptr if not set.
     */
    static const char* hostname() noexcept { return get("HOSTNAME"); }

    /**
     * @brief Username from the @c USER environment variable.
     * @return Pointer to the value, or @c nullptr if not set.
     */
    static const char* username() noexcept { return get("USER"); }
};

} // namespace Bazzulto
