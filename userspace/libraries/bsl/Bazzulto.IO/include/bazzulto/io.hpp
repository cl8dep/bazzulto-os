#pragma once
/**
 * @file io.hpp
 * @brief Bazzulto.IO — file, directory, and stream I/O (C++ API).
 *
 * Thin RAII wrappers and helpers over the C ABI in @c io.h.
 * All types live in the @c Bazzulto namespace.
 *
 * @code
 * #include <bazzulto/io.hpp>
 * using namespace Bazzulto;
 *
 * // Write a string to stdout.
 * Stream::stdout().write("Hello, Bazzulto!\n");
 *
 * // Read an entire file.
 * auto data = File::open("/system/config/disk-mounts").read_all();
 *
 * // List a directory.
 * for (const auto& entry : Directory::list("/system/bin/")) {
 *     Stream::stdout().write(entry.c_str());
 *     Stream::stdout().write("\n");
 * }
 * @endcode
 */

#include <bazzulto/io.h>
#include <stdint.h>
#include <stddef.h>

// Minimal no_std-compatible Vector / String substitute.
// In Bazzulto userspace these are backed by bz_mmap allocations; in hosted
// C++ programs they alias std::vector / std::string.
#ifdef __BAZZULTO_KERNEL_USERSPACE__
// Bare-metal: headers are provided by the BSL runtime (no std).
#  include <bazzulto/container.h>
#else
// Hosted (tests / tools running on Linux/macOS).
#  include <string>
#  include <vector>
namespace Bazzulto { using String = std::string; template<class T> using Vec = std::vector<T>; }
#endif

namespace Bazzulto {

// ---------------------------------------------------------------------------
// Stream — a thin wrapper around a raw file descriptor.
// ---------------------------------------------------------------------------

/**
 * @brief Unbuffered I/O stream over a file descriptor.
 *
 * Does NOT own the fd — callers are responsible for lifetime management.
 * The standard streams (stdin/stdout/stderr) are never closed.
 */
class Stream {
public:
    explicit constexpr Stream(int32_t fd) noexcept : fd_(fd) {}

    /** Standard streams — always valid. */
    static constexpr Stream stdin_stream()  noexcept { return Stream(0); }
    static constexpr Stream stdout_stream() noexcept { return Stream(1); }
    static constexpr Stream stderr_stream() noexcept { return Stream(2); }

    /** Write raw bytes.  Returns bytes written or negative errno. */
    int64_t write(const void* buf, size_t len) const noexcept {
        return bz_write(fd_, buf, len);
    }

    /** Write a NUL-terminated C string (excluding the terminator). */
    int64_t write(const char* s) const noexcept {
        size_t len = 0;
        while (s[len]) ++len;
        return bz_write(fd_, s, len);
    }

    /** Write a string followed by a newline. */
    int64_t writeln(const char* s) const noexcept {
        int64_t n = write(s);
        bz_write(fd_, "\n", 1);
        return n;
    }

    /** Read into @p buf.  Returns bytes read (0 = EOF) or negative errno. */
    int64_t read(void* buf, size_t len) const noexcept {
        return bz_read(fd_, buf, len);
    }

    int32_t fd() const noexcept { return fd_; }

private:
    int32_t fd_;
};

// ---------------------------------------------------------------------------
// File — RAII owner of an open file descriptor.
// ---------------------------------------------------------------------------

/**
 * @brief RAII file handle.
 *
 * Closes the underlying fd on destruction.  Non-copyable, moveable.
 *
 * @code
 * auto result = File::open("/system/config/disk-mounts");
 * if (!result.ok()) { ... error ... }
 * char buf[256];
 * int64_t n = result.value().read(buf, sizeof(buf));
 * @endcode
 */
class File {
public:
    /// Construct an invalid (closed) File.
    File() noexcept : fd_(-1) {}

    ~File() noexcept { close(); }

    // Non-copyable.
    File(const File&) = delete;
    File& operator=(const File&) = delete;

    // Moveable.
    File(File&& other) noexcept : fd_(other.fd_) { other.fd_ = -1; }
    File& operator=(File&& other) noexcept {
        if (this != &other) { close(); fd_ = other.fd_; other.fd_ = -1; }
        return *this;
    }

    /**
     * @brief Open an existing file for reading.
     * @return A File on success; check valid() before use.
     *         On failure errno() returns the negative error code.
     */
    static File open(const char* path) noexcept {
        size_t len = 0; while (path[len]) ++len;
        int64_t fd = bz_open(path, len, BZ_O_RDONLY, 0);
        return File(static_cast<int32_t>(fd < 0 ? fd : fd));
    }

    /**
     * @brief Create or truncate a file for writing.
     * @return A File on success; check valid() before use.
     */
    static File create(const char* path) noexcept {
        size_t len = 0; while (path[len]) ++len;
        int64_t fd = bz_creat(path, len);
        return File(static_cast<int32_t>(fd < 0 ? fd : fd));
    }

    /**
     * @brief Open with explicit flags and mode.
     * @param path      UTF-8 path.
     * @param flags     Combination of BZ_O_* flags.
     * @param mode      Permission bits (applied when BZ_O_CREAT is set).
     */
    static File open_with(const char* path, int32_t flags, uint32_t mode = 0) noexcept {
        size_t len = 0; while (path[len]) ++len;
        int64_t fd = bz_open(path, len, flags, mode);
        return File(static_cast<int32_t>(fd < 0 ? fd : fd));
    }

    /** @return true if the file was opened successfully. */
    bool valid() const noexcept { return fd_ >= 0; }

    /** @return The negative errno code if open failed, or 0 if valid. */
    int64_t error() const noexcept { return fd_ < 0 ? fd_ : 0; }

    int32_t fd() const noexcept { return fd_; }

    /** Read into @p buf.  Returns bytes read (0 = EOF) or negative errno. */
    int64_t read(void* buf, size_t len) const noexcept {
        return bz_read(fd_, buf, len);
    }

    /** Write @p buf to the file.  Returns bytes written or negative errno. */
    int64_t write(const void* buf, size_t len) const noexcept {
        return bz_write(fd_, buf, len);
    }

    /** Reposition the file offset. */
    int64_t seek(int64_t offset, int32_t whence = BZ_SEEK_SET) const noexcept {
        return bz_seek(fd_, offset, whence);
    }

    /** Flush pending writes to storage. */
    int64_t sync() const noexcept { return bz_fsync(fd_); }

    /** Close the file descriptor. Safe to call multiple times. */
    void close() noexcept {
        if (fd_ >= 0) { bz_close(fd_); fd_ = -1; }
    }

    /** Return a Stream view of this file (does not transfer ownership). */
    Stream as_stream() const noexcept { return Stream(fd_); }

private:
    explicit File(int32_t fd) noexcept : fd_(fd) {}
    int32_t fd_;
};

// ---------------------------------------------------------------------------
// Stat — file metadata query.
// ---------------------------------------------------------------------------

/**
 * @brief Query metadata for a file path.
 *
 * @param path  NUL-terminated UTF-8 path.
 * @param out   Written with metadata on success.
 * @return 0 on success, or a negative errno value.
 */
inline int64_t stat(const char* path, bz_stat_t& out) noexcept {
    size_t len = 0; while (path[len]) ++len;
    return bz_fstat(path, len, &out);
}

/**
 * @brief Return true if a file or directory exists at @p path.
 */
inline bool exists(const char* path) noexcept {
    bz_stat_t s{};
    return stat(path, s) == 0;
}

// ---------------------------------------------------------------------------
// Directory — listing helpers.
// ---------------------------------------------------------------------------

/**
 * @brief Directory listing utilities.
 *
 * Uses bz_open() + bz_getdents64() to enumerate directory entries.
 */
struct Directory {
    Directory() = delete;

    /**
     * @brief Return all entry names in @p dir_path.
     *
     * Entries "." and ".." are excluded.  Names are bare (no path prefix).
     *
     * @param dir_path  NUL-terminated path to the directory.
     * @return Vector of entry names, or empty on error.
     */
    static Vec<String> list(const char* dir_path) noexcept {
        Vec<String> result;
        File dir = File::open_with(dir_path, BZ_O_RDONLY | BZ_O_DIRECTORY);
        if (!dir.valid()) return result;

        uint8_t buf[4096];
        for (;;) {
            int64_t n = bz_getdents64(dir.fd(), buf, sizeof(buf));
            if (n <= 0) break;
            size_t offset = 0;
            while (offset < static_cast<size_t>(n)) {
                // Layout: ino(u64) off(u64) reclen(u16) type(u8) name(...)
                if (offset + 19 > static_cast<size_t>(n)) break;
                uint16_t reclen;
                __builtin_memcpy(&reclen, buf + offset + 16, 2);
                if (reclen == 0 || offset + reclen > static_cast<size_t>(n)) break;
                const char* name = reinterpret_cast<const char*>(buf + offset + 19);
                if (name[0] != '.' ||
                    (name[1] != '\0' && (name[1] != '.' || name[2] != '\0')))
                {
                    result.push_back(String(name));
                }
                offset += reclen;
            }
        }
        return result;
    }

    /**
     * @brief Return all entry names in @p dir_path ending with @p suffix.
     *
     * @param dir_path  NUL-terminated path.
     * @param suffix    NUL-terminated suffix to filter by.
     * @return Matching entry names (bare), or empty on error.
     */
    static Vec<String> list_with_suffix(const char* dir_path,
                                        const char* suffix) noexcept {
        Vec<String> all = list(dir_path);
        Vec<String> out;
        size_t slen = 0; while (suffix[slen]) ++slen;
        for (const auto& name : all) {
            size_t nlen = name.size();
            if (nlen >= slen &&
                name.compare(nlen - slen, slen, suffix) == 0)
            {
                out.push_back(name);
            }
        }
        return out;
    }
};

// ---------------------------------------------------------------------------
// Pipe — anonymous pipe pair.
// ---------------------------------------------------------------------------

/**
 * @brief RAII anonymous pipe pair.
 *
 * @code
 * auto result = Pipe::create();
 * if (!result.ok()) { ... error ... }
 * auto [read_end, write_end] = result.take();
 * @endcode
 */
struct Pipe {
    int32_t read_fd  = -1;
    int32_t write_fd = -1;

    /** Create a new pipe.  Returns a Pipe with read_fd = write_fd = -1 on failure. */
    static Pipe create() noexcept {
        int32_t fds[2] = { -1, -1 };
        bz_pipe(fds);
        return { fds[0], fds[1] };
    }

    bool valid() const noexcept { return read_fd >= 0 && write_fd >= 0; }

    /** Close both ends (idempotent). */
    void close() noexcept {
        if (read_fd  >= 0) { bz_close(read_fd);  read_fd  = -1; }
        if (write_fd >= 0) { bz_close(write_fd); write_fd = -1; }
    }

    Stream read_stream()  const noexcept { return Stream(read_fd); }
    Stream write_stream() const noexcept { return Stream(write_fd); }
};

} // namespace Bazzulto
