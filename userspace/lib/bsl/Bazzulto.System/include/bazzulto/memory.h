#pragma once
/** @file memory.h
 *  @brief Bazzulto.System — memory mapping C ABI
 */

#include <stdint.h>
#include <stddef.h>

/** @defgroup prot_flags Page Protection Flags
 *  @{
 */
#define BZ_PROT_READ   0x1  /**< Pages may be read. */
#define BZ_PROT_WRITE  0x2  /**< Pages may be written. */
#define BZ_PROT_EXEC   0x4  /**< Pages may be executed. */
/** @} */

/** @defgroup map_flags Mapping Flags
 *  @{
 */
#define BZ_MAP_ANONYMOUS 0x20  /**< Mapping is not backed by any file. */
#define BZ_MAP_PRIVATE   0x02  /**< Changes are private to this process. */
/** @} */

/**
 * @brief Map anonymous memory into the process address space.
 * @param addr    Requested base address hint, or 0 to let the kernel choose.
 * @param length  Number of bytes to map. Must be a multiple of the page size.
 * @param prot    Protection flags (combination of @c BZ_PROT_* values).
 * @param flags   Mapping flags (combination of @c BZ_MAP_* values).
 * @return Base address of the new mapping on success, or a negative errno value on failure.
 */
int64_t bz_mmap(uint64_t addr, uint64_t length, int32_t prot, int32_t flags);

/**
 * @brief Unmap a memory region.
 * @param addr    Base address of the region to unmap.
 * @param length  Number of bytes to unmap.
 * @return 0 on success, or a negative errno value on failure.
 */
int64_t bz_munmap(uint64_t addr, uint64_t length);
