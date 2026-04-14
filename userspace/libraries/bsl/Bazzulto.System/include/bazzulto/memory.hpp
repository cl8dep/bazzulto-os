#pragma once
// Bazzulto.System — memory mapping (C++ API)

#include <bazzulto/memory.h>
#include <cstdint>
#include <cstddef>

namespace Bazzulto {

/// RAII owner of an anonymous mmap region.
///
/// Automatically calls munmap on destruction. Not copyable; moveable.
class MappedRegion {
public:
    /// Map `length` bytes of anonymous read-write memory.
    /// Check `valid()` after construction.
    explicit MappedRegion(size_t length) noexcept
        : base_(nullptr), length_(length)
    {
        int64_t result = bz_mmap(0, length,
                                 BZ_PROT_READ | BZ_PROT_WRITE,
                                 BZ_MAP_ANONYMOUS | BZ_MAP_PRIVATE);
        if (result >= 0) {
            base_ = reinterpret_cast<void*>(static_cast<uintptr_t>(result));
        }
    }

    ~MappedRegion() noexcept {
        if (base_) {
            bz_munmap(reinterpret_cast<uint64_t>(base_), length_);
        }
    }

    // Non-copyable.
    MappedRegion(const MappedRegion&) = delete;
    MappedRegion& operator=(const MappedRegion&) = delete;

    // Moveable.
    MappedRegion(MappedRegion&& other) noexcept
        : base_(other.base_), length_(other.length_)
    {
        other.base_ = nullptr;
        other.length_ = 0;
    }

    MappedRegion& operator=(MappedRegion&& other) noexcept {
        if (this != &other) {
            if (base_) bz_munmap(reinterpret_cast<uint64_t>(base_), length_);
            base_ = other.base_;
            length_ = other.length_;
            other.base_ = nullptr;
            other.length_ = 0;
        }
        return *this;
    }

    bool valid() const noexcept { return base_ != nullptr; }
    void* data()  const noexcept { return base_; }
    size_t size() const noexcept { return length_; }

    /// Typed access to the mapped memory.
    template <typename T>
    T* as() const noexcept { return static_cast<T*>(base_); }

private:
    void*  base_;
    size_t length_;
};

/// Map anonymous read-write memory. Returns base address or negative errno.
inline int64_t mmap(uint64_t addr, uint64_t length, int32_t prot, int32_t flags) noexcept {
    return bz_mmap(addr, length, prot, flags);
}

/// Unmap a region. Returns 0 or negative errno.
inline int64_t munmap(uint64_t addr, uint64_t length) noexcept {
    return bz_munmap(addr, length);
}

} // namespace Bazzulto
