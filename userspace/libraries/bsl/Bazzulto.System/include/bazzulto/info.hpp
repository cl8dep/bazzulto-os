#pragma once
/**
 * @file info.hpp
 * @brief Bazzulto.System — OS and hardware information (C++ API).
 *
 * OS identity strings are sourced from the kernel at runtime via sys_uname
 * (slot 41).  Results are cached in process-local statics so repeated calls
 * are free after the first call.
 *
 * Hardware stats (uptime, memory) come from sys_sysinfo (slot 42) on each call.
 *
 * @code
 * #include <bazzulto/info.hpp>
 *
 * auto name = Bazzulto::Info::os_name();       // e.g. "Bazzulto"
 * auto ver  = Bazzulto::Info::kernel_version(); // e.g. "0.1.0"
 * auto mem  = Bazzulto::Info::memory_total();   // bytes
 * @endcode
 */

#include <stdint.h>
#include <stddef.h>

/**
 * @defgroup bz_uname sys_uname
 * @brief Low-level OS identity syscall.  Prefer the Bazzulto::Info C++ API.
 *
 * Buffer layout (390 bytes): 6 fields × 65 bytes, each NUL-terminated.
 *   buf[  0.. 65)  sysname    — OS name         (e.g. "Bazzulto")
 *   buf[ 65..130)  nodename   — hostname         (e.g. "bazzulto")
 *   buf[130..195)  release    — kernel release   (e.g. "0.1.0")
 *   buf[195..260)  version    — build string     (e.g. "Bazzulto 0.1.0 (AArch64)")
 *   buf[260..325)  machine    — ISA              (e.g. "aarch64")
 *   buf[325..390)  domainname — NIS domain name  (empty string)
 * @{
 */
#define BZ_UNAME_FIELD_LEN  65u   /**< Length of each uname field in bytes. */
#define BZ_UNAME_BUF_LEN   390u   /**< Total uname buffer size (6 × 65). */

/** Field indices into the uname buffer. */
#define BZ_UNAME_SYSNAME     0u   /**< OS name field index. */
#define BZ_UNAME_NODENAME    1u   /**< Hostname field index. */
#define BZ_UNAME_RELEASE     2u   /**< Kernel release field index. */
#define BZ_UNAME_VERSION     3u   /**< Build description field index. */
#define BZ_UNAME_MACHINE     4u   /**< ISA field index. */
#define BZ_UNAME_DOMAINNAME  5u   /**< NIS domain name field index. */
/** @} */

/**
 * @defgroup bz_sysinfo sys_sysinfo
 * @brief System statistics buffer layout.
 *
 * Buffer layout: 4 × uint64_t (little-endian, naturally aligned).
 *   buf[0]  uptime_seconds — monotonic seconds since boot
 *   buf[1]  total_ram      — total physical RAM in bytes
 *   buf[2]  free_ram       — free physical RAM in bytes
 *   buf[3]  process_count  — number of live processes
 * @{
 */
#define BZ_SYSINFO_BUF_WORDS  4u  /**< Number of uint64_t words in the sysinfo buffer. */

/** Word indices into the sysinfo buffer. */
#define BZ_SYSINFO_UPTIME   0u   /**< Uptime in seconds. */
#define BZ_SYSINFO_TOTAL    1u   /**< Total RAM in bytes. */
#define BZ_SYSINFO_FREE     2u   /**< Free RAM in bytes. */
#define BZ_SYSINFO_PROCS    3u   /**< Live process count. */
/** @} */

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Fill a POSIX-compatible utsname buffer.
 * @param buf  Output buffer, must be at least BZ_UNAME_BUF_LEN bytes.
 * @return 0 on success, negative errno on failure.
 */
int64_t bz_uname(uint8_t *buf);

/**
 * @brief Fill a sysinfo buffer with system-wide statistics.
 * @param buf  Output buffer of BZ_SYSINFO_BUF_WORDS × uint64_t.
 * @return 0 on success, negative errno on failure.
 */
int64_t bz_sysinfo(uint64_t *buf);

/** Return the PID of the calling process. */
int64_t bz_getpid(void);

/** Return the PPID of the calling process. */
int64_t bz_getppid(void);

#ifdef __cplusplus
} /* extern "C" */

namespace Bazzulto {

/**
 * @brief OS and hardware information.
 *
 * All methods are static.  OS identity strings are fetched from the kernel
 * once per process and cached; hardware stats are fetched on each call.
 */
struct Info {
    Info() = delete;

    /**
     * @brief Operating system name (e.g. @c "Bazzulto").
     *
     * Sourced from sys_uname field @c sysname.  Cached after the first call.
     * @return Pointer to a process-local NUL-terminated string.
     */
    static const char* os_name() noexcept { return uname_field(BZ_UNAME_SYSNAME); }

    /**
     * @brief OS build description (e.g. @c "Bazzulto 0.1.0 (AArch64)").
     *
     * Sourced from sys_uname field @c version.  Cached after the first call.
     */
    static const char* os_version() noexcept { return uname_field(BZ_UNAME_VERSION); }

    /**
     * @brief Kernel release version (e.g. @c "0.1.0").
     *
     * Sourced from sys_uname field @c release.  Cached after the first call.
     */
    static const char* kernel_version() noexcept { return uname_field(BZ_UNAME_RELEASE); }

    /**
     * @brief CPU architecture (e.g. @c "aarch64").
     *
     * Sourced from sys_uname field @c machine.  Cached after the first call.
     */
    static const char* arch() noexcept { return uname_field(BZ_UNAME_MACHINE); }

    /** PID of the calling process (syscall-backed). */
    static int32_t pid()  noexcept { return static_cast<int32_t>(bz_getpid());  }
    /** PPID of the calling process (syscall-backed). */
    static int32_t ppid() noexcept { return static_cast<int32_t>(bz_getppid()); }

    /**
     * @brief Total physical RAM in bytes — from sys_sysinfo.
     * @return Total RAM, or 0 if the syscall fails.
     */
    static uint64_t memory_total() noexcept {
        uint64_t buf[BZ_SYSINFO_BUF_WORDS] = {};
        if (bz_sysinfo(buf) == 0) return buf[BZ_SYSINFO_TOTAL];
        return 0;
    }

    /**
     * @brief Free (available) physical RAM in bytes — from sys_sysinfo.
     * @return Free RAM, or 0 if the syscall fails.
     */
    static uint64_t memory_free() noexcept {
        uint64_t buf[BZ_SYSINFO_BUF_WORDS] = {};
        if (bz_sysinfo(buf) == 0) return buf[BZ_SYSINFO_FREE];
        return 0;
    }

    /**
     * @brief System uptime in seconds — from sys_sysinfo.
     * @return Uptime, or 0 if the syscall fails.
     */
    static uint64_t uptime_seconds() noexcept {
        uint64_t buf[BZ_SYSINFO_BUF_WORDS] = {};
        if (bz_sysinfo(buf) == 0) return buf[BZ_SYSINFO_UPTIME];
        return 0;
    }

    /**
     * @brief Number of live processes — from sys_sysinfo.
     * @return Process count, or 0 if the syscall fails.
     */
    static uint64_t process_count() noexcept {
        uint64_t buf[BZ_SYSINFO_BUF_WORDS] = {};
        if (bz_sysinfo(buf) == 0) return buf[BZ_SYSINFO_PROCS];
        return 0;
    }

    /// CPU brand string — not yet provided by the kernel.
    static const char* cpu_brand() noexcept { return nullptr; }
    /// CPU count — not yet provided by the kernel.
    static uint32_t    cpu_count() noexcept { return 0; }

private:
    // uname cache — function-local statics, initialized on first call.
    // Function-local static variables are guaranteed to be initialized exactly
    // once (C++11 §6.7) so this is safe in single-threaded userspace.
    static const char* uname_field(unsigned index) noexcept {
        static uint8_t buf[BZ_UNAME_BUF_LEN] = {};
        static bool    loaded = false;
        if (!loaded) {
            bz_uname(buf);
            loaded = true;
        }
        return reinterpret_cast<const char*>(buf + index * BZ_UNAME_FIELD_LEN);
    }
};

} // namespace Bazzulto

#endif /* __cplusplus */
