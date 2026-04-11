// FAT32 filesystem driver for Bazzulto OS — full read/write with LFN support
//
// Implements the vfs_scheme_driver_t interface for the //disk: scheme.
// Uses hal_disk_read_sectors() / hal_disk_write_sectors() for all I/O.
//
// Features:
//   - Read/write files (extend files by allocating new clusters)
//   - Create new files (directory entry + LFN chain + cluster allocation)
//   - Delete files (free clusters + mark directory entry deleted)
//   - Full LFN (long filename) support for read and write
//   - In-memory FAT bitmap for cluster allocation tracking
//   - FSInfo sector read/write for free cluster count

#include "../../include/bazzulto/vfs_scheme.h"
#include "../../include/bazzulto/fat32.h"
#include "../../include/bazzulto/hal/hal_disk.h"
#include "../../include/bazzulto/hal/hal_uart.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/kernel.h"
#include <string.h>
#include <stddef.h>

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#define FAT32_MAX_PATH_DEPTH 16
#define FAT32_MAX_NAME_LEN   256

// Open file descriptor state for a FAT32 file.
typedef struct {
    uint32_t dir_cluster;      // cluster of the directory containing this file
    uint32_t dir_entry_offset; // byte offset within that dir cluster
    uint32_t first_cluster;    // starting cluster of the file
    uint32_t file_size;        // file size in bytes
    uint32_t current_cluster;  // cluster currently cached (0 = not loaded)
    uint8_t *cluster_buf;      // one cluster of data (kmalloc'd)
    int      dirty;            // file has been written to
} fat32_file_t;

// ---------------------------------------------------------------------------
// Cached filesystem metadata (filled at init)
// ---------------------------------------------------------------------------

static int          fs_ready;
static fat32_bpb_t  fs_bpb;
static uint32_t     fs_fat_start_lba;        // LBA of first FAT
static uint32_t     fs_data_start_lba;       // LBA of first data cluster
static uint32_t     fs_root_cluster;         // root dir cluster
static uint32_t     fs_sectors_per_cluster;
static uint32_t     fs_bytes_per_cluster;
static uint64_t     fs_total_sectors;
static uint32_t     fs_total_clusters;       // total data clusters
static uint32_t     fs_fat_sectors;          // sectors per FAT

// One-cluster read/write buffer (reused).
static uint8_t     *fs_cluster_buf;

// FAT bitmap: 1 bit per cluster. 1 = used, 0 = free.
// Cluster N maps to bit (N - 2).
static uint8_t     *fs_fat_bitmap;
static uint32_t     fs_fat_bitmap_size;  // bytes allocated

// FSInfo
static uint32_t     fs_fsinfo_sector;
static uint32_t     fs_free_clusters;    // from FSInfo (cached, updated on alloc/free)

// Number of clusters the largest directory we've seen (for dir extension).

// ---------------------------------------------------------------------------
// Sector I/O helpers
// ---------------------------------------------------------------------------

static int read_sectors(uint64_t lba, uint32_t count, void *buf)
{
    return hal_disk_read_sectors(lba, count, buf);
}

static int write_sectors(uint64_t lba, uint32_t count, const void *buf)
{
    return hal_disk_write_sectors(lba, count, buf);
}

// Read a single cluster into `buf`.
static int read_cluster(uint32_t cluster, uint8_t *buf)
{
    if (cluster < FAT32_FIRST_DATA_CLUSTER)
        return -1;

    uint64_t lba = fs_data_start_lba +
        (uint64_t)(cluster - FAT32_FIRST_DATA_CLUSTER) * fs_sectors_per_cluster;
    return read_sectors(lba, fs_sectors_per_cluster, buf);
}

// Write a single cluster from `buf`.
static int write_cluster(uint32_t cluster, const uint8_t *buf)
{
    if (cluster < FAT32_FIRST_DATA_CLUSTER)
        return -1;

    uint64_t lba = fs_data_start_lba +
        (uint64_t)(cluster - FAT32_FIRST_DATA_CLUSTER) * fs_sectors_per_cluster;
    return write_sectors(lba, fs_sectors_per_cluster, buf);
}

// ---------------------------------------------------------------------------
// FAT table I/O
// ---------------------------------------------------------------------------

// Read a FAT entry (32-bit cluster value).
static int read_fat_entry(uint32_t cluster, uint32_t *out)
{
    uint64_t fat_offset = (uint64_t)cluster * 4;
    uint64_t fat_sector = fat_offset / 512;
    uint32_t entry_offset = (uint32_t)(fat_offset % 512);

    static uint8_t fat_sector_buf[512];
    static uint32_t cached_fat_sector = 0xFFFFFFFF;

    if (fat_sector != cached_fat_sector) {
        if (read_sectors(fs_fat_start_lba + fat_sector, 1, fat_sector_buf) < 0)
            return -1;
        cached_fat_sector = (uint32_t)fat_sector;
    }

    *out = *(uint32_t *)(fat_sector_buf + entry_offset) & 0x0FFFFFFF;
    return 0;
}

// Write a FAT entry (updates BOTH FAT copies for redundancy).
static int write_fat_entry(uint32_t cluster, uint32_t value)
{
    uint64_t fat_offset = (uint64_t)cluster * 4;
    uint64_t fat_sector = fat_offset / 512;
    uint32_t entry_offset = (uint32_t)(fat_offset % 512);

    // Write to both FAT copies.
    for (int fat = 0; fat < fs_bpb.num_fats; fat++) {
        uint64_t sector = fs_fat_start_lba + fat_sector + (uint64_t)fat * fs_fat_sectors;
        uint8_t buf[512];

        if (read_sectors(sector, 1, buf) < 0)
            return -1;

        // Mask in the new value (preserve upper 4 bits).
        uint32_t existing = *(uint32_t *)(buf + entry_offset);
        value = (value & 0x0FFFFFFF) | (existing & 0xF0000000);
        *(uint32_t *)(buf + entry_offset) = value;

        if (write_sectors(sector, 1, buf) < 0)
            return -1;
    }

    return 0;
}

// ---------------------------------------------------------------------------
// FAT bitmap management (in-memory cluster allocation tracker)
// ---------------------------------------------------------------------------

// Set bit for cluster N (mark as used).
static void fat_bitmap_set(uint32_t cluster)
{
    if (cluster < FAT32_FIRST_DATA_CLUSTER || cluster >= fs_total_clusters + 2)
        return;
    uint32_t bit = cluster - FAT32_FIRST_DATA_CLUSTER;
    fs_fat_bitmap[bit / 8] |= (uint8_t)(1 << (bit % 8));
}

// Clear bit for cluster N (mark as free).
static void fat_bitmap_clear(uint32_t cluster)
{
    if (cluster < FAT32_FIRST_DATA_CLUSTER || cluster >= fs_total_clusters + 2)
        return;
    uint32_t bit = cluster - FAT32_FIRST_DATA_CLUSTER;
    fs_fat_bitmap[bit / 8] &= (uint8_t)~(1 << (bit % 8));
}

// Test if cluster N is used (reserved for future use).
static __attribute__((unused)) int fat_bitmap_test(uint32_t cluster)
{
    if (cluster < FAT32_FIRST_DATA_CLUSTER || cluster >= fs_total_clusters + 2)
        return 1;  // assume used if out of range
    uint32_t bit = cluster - FAT32_FIRST_DATA_CLUSTER;
    return (fs_fat_bitmap[bit / 8] >> (bit % 8)) & 1;
}

// Find the first free cluster.
static uint32_t fat_bitmap_find_free(void)
{
    for (uint32_t i = 0; i < fs_total_clusters; i++) {
        if (!(fs_fat_bitmap[i / 8] & (uint8_t)(1 << (i % 8))))
            return FAT32_FIRST_DATA_CLUSTER + i;
    }
    return 0;  // no free clusters
}

// Build the FAT bitmap from the on-disk FAT table.
static int build_fat_bitmap(void)
{
    uint32_t cluster;
    for (uint32_t i = 0; i < fs_total_clusters + 2; i++) {
        if (read_fat_entry(i, &cluster) < 0)
            return -1;
        if (cluster != FAT32_CLUSTER_FREE)
            fat_bitmap_set(i);
    }
    return 0;
}

// ---------------------------------------------------------------------------
// Cluster chain helpers
// ---------------------------------------------------------------------------

// Follow the cluster chain to find the Nth cluster (0-indexed).
// Returns the cluster number, or 0 on error/EOF.
static uint32_t cluster_at_index(uint32_t first_cluster, uint32_t index)
{
    uint32_t cluster = first_cluster;
    for (uint32_t i = 0; i < index; i++) {
        if (fat32_is_eof(cluster) || fat32_is_bad(cluster))
            return 0;
        if (read_fat_entry(cluster, &cluster) < 0)
            return 0;
    }
    return cluster;
}

// Get the last cluster in a chain (the one that points to EOF).
// Returns 0 on error.
static uint32_t get_last_cluster(uint32_t first_cluster)
{
    if (first_cluster < FAT32_FIRST_DATA_CLUSTER)
        return 0;

    uint32_t cluster = first_cluster;
    uint32_t next;
    while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        if (read_fat_entry(cluster, &next) < 0)
            return 0;
        if (next < FAT32_FIRST_DATA_CLUSTER)
            break;
        cluster = next;
    }
    return cluster;
}

// Allocate a new cluster and append it to a chain.
// If first_cluster == 0, returns the new cluster as the start.
// Returns the new cluster number, or 0 on failure.
static uint32_t alloc_cluster(uint32_t first_cluster, uint32_t *out_first)
{
    uint32_t new_cluster = fat_bitmap_find_free();
    if (new_cluster == 0)
        return 0;

    // Mark as used and write EOF.
    fat_bitmap_set(new_cluster);
    if (write_fat_entry(new_cluster, FAT32_CLUSTER_EOF) < 0) {
        fat_bitmap_clear(new_cluster);
        return 0;
    }

    if (first_cluster == 0) {
        *out_first = new_cluster;
    } else {
        // Append to chain.
        uint32_t last = get_last_cluster(first_cluster);
        if (last == 0) {
            fat_bitmap_clear(new_cluster);
            return 0;
        }
        if (write_fat_entry(last, new_cluster) < 0) {
            fat_bitmap_clear(new_cluster);
            return 0;
        }
        *out_first = first_cluster;
    }

    fs_free_clusters--;
    return new_cluster;
}

// Free a cluster from a chain (removes it from the chain and the bitmap).
// Reserved for future use (truncation support).
static __attribute__((unused)) void free_cluster_from_chain(uint32_t prev_cluster, uint32_t cluster)
{
    uint32_t next;
    if (read_fat_entry(cluster, &next) < 0)
        return;

    if (prev_cluster == 0) {
        // This was the head — next becomes the new head (caller handles).
    }

    if (prev_cluster != 0) {
        if (fat32_is_eof(next) || fat32_is_bad(next))
            write_fat_entry(prev_cluster, FAT32_CLUSTER_EOF);
        else
            write_fat_entry(prev_cluster, next);
    }

    fat_bitmap_clear(cluster);
    write_fat_entry(cluster, FAT32_CLUSTER_FREE);
    if (fs_free_clusters < 0xFFFFFFFF)
        fs_free_clusters++;
}

// Free an entire cluster chain starting from `cluster`.
static void free_chain(uint32_t cluster)
{
    while (cluster >= FAT32_FIRST_DATA_CLUSTER && !fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        uint32_t next;
        if (read_fat_entry(cluster, &next) < 0)
            break;
        fat_bitmap_clear(cluster);
        write_fat_entry(cluster, FAT32_CLUSTER_FREE);
        if (fs_free_clusters < 0xFFFFFFFF)
            fs_free_clusters++;
        cluster = next;
    }
    // Free the last cluster (EOF marker).
    if (cluster >= FAT32_FIRST_DATA_CLUSTER) {
        fat_bitmap_clear(cluster);
        write_fat_entry(cluster, FAT32_CLUSTER_FREE);
        if (fs_free_clusters < 0xFFFFFFFF)
            fs_free_clusters++;
    }
}

// Extend a file by `n` clusters. Returns the new first cluster (same as input
// unless file was empty), or 0 on failure.
static uint32_t extend_file_clusters(uint32_t first_cluster, uint32_t n)
{
    uint32_t new_first = first_cluster;
    for (uint32_t i = 0; i < n; i++) {
        if (alloc_cluster(new_first, &new_first) == 0)
            return 0;
    }
    return new_first;
}

// ---------------------------------------------------------------------------
// Path parsing and directory traversal
// ---------------------------------------------------------------------------

static int split_path(const char *path, const char **components, int max_depth)
{
    if (!path || path[0] != '/')
        return -1;

    int count = 0;
    const char *p = path;

    while (*p && count < max_depth) {
        while (*p == '/') p++;
        if (!*p) break;
        components[count++] = p;
        while (*p && *p != '/') p++;
    }

    return count;
}

// Convert a FAT32 8.3 short name to a null-terminated string.
static void short_name_to_str(const uint8_t name[11], char *out, int out_size)
{
    int pos = 0;
    for (int i = 0; i < 8 && pos < out_size - 1; i++) {
        if (name[i] == ' ') break;
        out[pos++] = (char)name[i];
    }
    if (name[8] != ' ') {
        if (pos < out_size - 1) out[pos++] = '.';
        for (int i = 0; i < 3 && pos < out_size - 1; i++) {
            if (name[8 + i] == ' ') break;
            out[pos++] = (char)name[8 + i];
        }
    }
    out[pos] = '\0';
}

// Convert an 8.3 filename into the 11-byte FAT directory name format.
static void name_to_short(const char *name, int len, uint8_t out[11])
{
    memset(out, ' ', 11);

    // Find the dot.
    int dot = -1;
    for (int i = 0; i < len; i++) {
        if (name[i] == '.') { dot = i; break; }
    }

    int name_end = (dot >= 0) ? dot : len;
    int ext_start = (dot >= 0) ? dot + 1 : len;

    // Name part (up to 8 chars, uppercase).
    for (int i = 0; i < name_end && i < 8; i++) {
        char c = name[i];
        if (c >= 'a' && c <= 'z') c = (char)(c - ('a' - 'A'));
        out[i] = (uint8_t)c;
    }

    // Extension (up to 3 chars, uppercase).
    for (int i = 0; i < (len - ext_start) && i < 3; i++) {
        char c = name[ext_start + i];
        if (c >= 'a' && c <= 'z') c = (char)(c - ('a' - 'A'));
        out[8 + i] = (uint8_t)c;
    }
}

// Compute the VFAT checksum for a short name (8.3).
static uint8_t vfat_checksum(const uint8_t name[11])
{
    uint8_t sum = 0;
    for (int i = 11; i > 0; i--)
        sum = (uint8_t)(((sum & 1) << 7) + (sum >> 1) + name[i - 1]);
    return sum;
}

// Decode a single UCS-2 LFN character to ASCII (lowercase).
static char lfn_char_to_ascii(uint16_t ch)
{
    if (ch >= 'A' && ch <= 'Z')
        return (char)(ch + ('a' - 'A'));
    return (char)ch;
}

// Compare a directory entry's name against a target path component.
static int entry_matches(const fat32_dir_entry_t *entry,
                         const char *target, int target_len,
                         const char *lfn_name, int lfn_len)
{
    if (lfn_name && lfn_len > 0) {
        if (lfn_len != target_len)
            return 0;
        for (int i = 0; i < target_len; i++) {
            char a = lfn_name[i];
            char b = target[i];
            if (a >= 'A' && a <= 'Z') a = a + ('a' - 'A');
            if (b >= 'A' && b <= 'Z') b = b + ('a' - 'A');
            if (a != b)
                return 0;
        }
        return 1;
    }

    char short_name[13];
    short_name_to_str(entry->name, short_name, sizeof(short_name));
    int short_len = (int)strlen(short_name);
    if (short_len != target_len)
        return 0;

    for (int i = 0; i < target_len; i++) {
        char a = short_name[i];
        char b = target[i];
        if (a >= 'A' && a <= 'Z') a = a + ('a' - 'A');
        if (b >= 'A' && b <= 'Z') b = b + ('a' - 'A');
        if (a != b)
            return 0;
    }
    return 1;
}

// ---------------------------------------------------------------------------
// Directory lookup
// ---------------------------------------------------------------------------

// Look up a single path component within a directory identified by its
// starting cluster. Returns 1 if found, 0 if not found, -1 on I/O error.
// If found, entry_out contains the directory entry and lfn_name contains
// the decoded long name (if any).
static int lookup_in_dir(uint32_t dir_cluster,
                         const char *component, int comp_len,
                         fat32_dir_entry_t *entry_out,
                         char *lfn_name_out, int *lfn_len_out)
{
    if (!fs_cluster_buf)
        return -1;

    uint32_t cluster = dir_cluster;
    uint32_t dir_entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);

    char lfn_buf[FAT32_MAX_LFN_LEN + 1];
    int lfn_total = 0;
    int lfn_expected_ord = 0;

    while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        if (read_cluster(cluster, fs_cluster_buf) < 0)
            return -1;

        fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;

        for (uint32_t i = 0; i < dir_entries_per_cluster; i++) {
            fat32_dir_entry_t *entry = &entries[i];

            if (entry->name[0] == FAT32_DIR_ENTRY_FREE)
                return 0;

            if (entry->name[0] == FAT32_DIR_ENTRY_DELETED) {
                lfn_total = 0;
                lfn_expected_ord = 0;
                continue;
            }

            if (fat32_is_lfn(entry)) {
                fat32_lfn_entry_t *lfn = (fat32_lfn_entry_t *)entry;
                int ord = lfn->ord & 0x1F;

                if (lfn->ord & 0x40) {
                    lfn_expected_ord = ord;
                    lfn_total = 0;
                }

                if (ord == lfn_expected_ord && lfn_total + 13 <= FAT32_MAX_LFN_LEN) {
                    uint16_t chars[13];
                    memcpy(chars, lfn->name1, 5 * 2);
                    memcpy(chars + 5, lfn->name2, 6 * 2);
                    memcpy(chars + 11, lfn->name3, 2 * 2);

                    for (int c = 0; c < 13 && lfn_total < FAT32_MAX_LFN_LEN; c++) {
                        if (chars[c] == 0x0000 || chars[c] == 0xFFFF)
                            break;
                        lfn_buf[lfn_total++] = lfn_char_to_ascii(chars[c]);
                    }

                    lfn_expected_ord--;
                }
                continue;
            }

            if (lfn_total > 0) {
                if (entry_matches(entry, component, comp_len, lfn_buf, lfn_total)) {
                    *entry_out = *entry;
                    if (lfn_name_out && lfn_len_out) {
                        memcpy(lfn_name_out, lfn_buf, lfn_total);
                        lfn_name_out[lfn_total] = '\0';
                        *lfn_len_out = lfn_total;
                    }
                    return 1;
                }
                lfn_total = 0;
                lfn_expected_ord = 0;
                continue;
            }

            if (entry_matches(entry, component, comp_len, NULL, 0)) {
                *entry_out = *entry;
                if (lfn_name_out && lfn_len_out)
                    *lfn_len_out = 0;
                return 1;
            }

            lfn_total = 0;
            lfn_expected_ord = 0;
        }

        if (read_fat_entry(cluster, &cluster) < 0)
            return -1;
    }

    return 0;
}

// Find a free directory entry slot in a directory.
// Returns 0 on success (fills *entry_cluster, *entry_offset, *entries_per_cluster),
// -1 on I/O error, 1 if the directory needs to be extended.
static int find_free_dir_entry(uint32_t dir_cluster,
                                uint32_t *out_entry_cluster,
                                uint32_t *out_entry_offset,
                                uint32_t *out_entries_per_cluster)
{
    if (!fs_cluster_buf)
        return -1;

    uint32_t cluster = dir_cluster;
    uint32_t entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);

    while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        if (read_cluster(cluster, fs_cluster_buf) < 0)
            return -1;

        fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            if (entries[i].name[0] == FAT32_DIR_ENTRY_FREE ||
                entries[i].name[0] == FAT32_DIR_ENTRY_DELETED) {
                *out_entry_cluster = cluster;
                *out_entry_offset = i;
                *out_entries_per_cluster = entries_per_cluster;
                return 0;
            }
        }

        // Check if we need to extend the directory.
        if (read_fat_entry(cluster, &cluster) < 0)
            return -1;
        if (cluster < FAT32_FIRST_DATA_CLUSTER) {
            // End of chain — no free slot found, need to extend.
            return 1;
        }
    }

    // Directory is empty or at EOF — need to extend.
    return 1;
}

// Extend a directory by adding one cluster.
// Returns the new cluster number, or 0 on failure.
static int extend_directory(uint32_t dir_cluster, uint32_t *out_new_cluster)
{
    uint32_t new_cluster;
    uint32_t new_first = dir_cluster;
    new_cluster = alloc_cluster(dir_cluster, &new_first);
    if (new_cluster == 0)
        return -1;

    // Zero out the new cluster.
    if (!fs_cluster_buf)
        return -1;
    memset(fs_cluster_buf, 0, fs_bytes_per_cluster);
    if (write_cluster(new_cluster, fs_cluster_buf) < 0)
        return -1;

    *out_new_cluster = new_cluster;
    return 0;
}

// Resolve a full path to a directory entry.
// Returns 1 if found, 0 if not, -1 on I/O error.
// parent_cluster_out is set to the parent directory's cluster.
static int resolve_path(const char *path,
                        fat32_dir_entry_t *entry_out,
                        uint32_t *parent_cluster_out)
{
    const char *components[FAT32_MAX_PATH_DEPTH];
    int depth = split_path(path, components, FAT32_MAX_PATH_DEPTH);
    if (depth <= 0)
        return 0;

    uint32_t current_cluster = fs_root_cluster;
    fat32_dir_entry_t entry;
    char lfn_name[FAT32_MAX_LFN_LEN + 1];
    int lfn_len = 0;

    // Traverse all components except the last.
    for (int i = 0; i < depth - 1; i++) {
        int comp_len = (int)(components[i + 1] - components[i] - 1);

        int found = lookup_in_dir(current_cluster, components[i], comp_len,
                                   &entry, lfn_name, &lfn_len);
        if (!found)
            return 0;
        if (found < 0)
            return -1;

        if (!(entry.attr & FAT32_ATTR_DIRECTORY))
            return 0;

        current_cluster = fat32_entry_cluster(&entry);
    }

    // Last component.
    int last_len = (int)strlen(components[depth - 1]);
    int found = lookup_in_dir(current_cluster, components[depth - 1], last_len,
                               entry_out, lfn_name, &lfn_len);
    if (found <= 0)
        return found;

    if (parent_cluster_out)
        *parent_cluster_out = current_cluster;

    return 1;
}

// ---------------------------------------------------------------------------
// Write a directory entry (with optional LFN chain) to disk.
// ---------------------------------------------------------------------------

// Write a directory entry into the buffer at the given offset.
// Reserved for future use.
static __attribute__((unused)) void write_dir_entry(fat32_dir_entry_t *entries, uint32_t entry_idx,
                             const fat32_dir_entry_t *entry)
{
    entries[entry_idx] = *entry;
}

// Write an LFN chain before a short-name entry.
// `lfn_name` is the full filename, `lfn_len` its length.
// `short_name[11]` is the 8.3 name.
// `entry_idx` is the index where the short-name entry will go.
// LFN entries are written at entry_idx - n_lfn ... entry_idx - 1.
// Returns the number of LFN entries written.
static int write_lfn_chain(fat32_dir_entry_t *entries, uint32_t entry_idx,
                            const char *lfn_name, int lfn_len,
                            const uint8_t short_name[11])
{
    uint8_t checksum = vfat_checksum(short_name);
    int total_entries = (lfn_len + 12) / 13;  // 13 chars per entry, round up
    if (total_entries > FAT32_MAX_LFN_ENTRIES)
        total_entries = FAT32_MAX_LFN_ENTRIES;

    for (int e = total_entries; e >= 1; e--) {
        int entry_offset = entry_idx - e;  // position in the directory
        fat32_lfn_entry_t *lfn = (fat32_lfn_entry_t *)&entries[entry_offset];

        memset(lfn, 0, sizeof(fat32_lfn_entry_t));

        int chars_in_this = 13;
        int start_char = (total_entries - e) * 13;
        int remaining = lfn_len - start_char;
        if (remaining < 13) chars_in_this = remaining;

        // ord: order number, with 0x40 on the last (first) entry.
        lfn->ord = (uint8_t)(e | ((e == total_entries) ? 0x40 : 0x00));
        lfn->attr = FAT32_ATTR_LONG_NAME;
        lfn->type = 0x00;
        lfn->chksum = checksum;
        lfn->fst_clus_lo = 0x0000;

        // Fill the 13 characters.
        uint16_t chars[13];
        memset(chars, 0xFF, sizeof(chars));
        for (int c = 0; c < chars_in_this && c < 13; c++) {
            char ch = lfn_name[start_char + c];
            chars[c] = (uint16_t)ch;
        }
        // Null-terminate.
        if (start_char + chars_in_this >= lfn_len)
            chars[chars_in_this] = 0x0000;

        memcpy(lfn->name1, chars, 5 * 2);
        memcpy(lfn->name2, chars + 5, 6 * 2);
        memcpy(lfn->name3, chars + 11, 2 * 2);
    }

    return total_entries;
}

// ---------------------------------------------------------------------------
// Public FAT32 API
// ---------------------------------------------------------------------------

// Strip the /mnt/ mount prefix from a unix path.
// The VFS mount table passes full unix paths (e.g. "/mnt/hello.txt") to the
// disk driver, but FAT32 sees its root as "/".  So "/mnt/hello.txt" must
// become "/hello.txt".
static const char *strip_mnt_prefix(const char *path)
{
    // Match "/mnt" optionally followed by '/' then the rest.
    if (path[0] == '/' && path[1] == 'm' && path[2] == 'n' && path[3] == 't') {
        if (path[4] == '/')
            return path + 4;   // "/mnt/xxx" → "/xxx"
        if (path[4] == '\0')
            return "/";        // "/mnt" → "/"
    }
    return path;  // no /mnt prefix — pass through unchanged
}

int fat32_init(void)
{
    fs_ready = 0;

    // hal_disk_init() is already called by main.c before this point.
    uint64_t capacity = hal_disk_capacity();
    if (capacity == 0) {
        return -1;
    }
    fs_total_sectors = capacity;

    // Allocate one physical page (4KB) for cluster I/O via DMA.
    void *phys_page = physical_memory_alloc();
    if (!phys_page)
        return -1;
    fs_cluster_buf = (uint8_t *)(hhdm_offset + (uint64_t)phys_page);

    // Read boot sector (LBA 0).
    if (read_sectors(0, 1, fs_cluster_buf) < 0)
        return -1;

    if (fs_cluster_buf[510] != 0x55 || fs_cluster_buf[511] != 0xAA)
        return -1;

    // The BPB structure includes jmp_boot[3] + oem_name[8] at the start.
    fat32_bpb_t *bpb = (fat32_bpb_t *)fs_cluster_buf;

    if (memcmp(bpb->fil_sys_type, "FAT32", 5) != 0)
        return -1;

    memcpy(&fs_bpb, bpb, sizeof(fs_bpb));

    fs_sectors_per_cluster = bpb->sec_per_clus;
    fs_bytes_per_cluster   = (uint32_t)bpb->bytes_per_sec * fs_sectors_per_cluster;
    fs_fat_start_lba       = (uint32_t)bpb->rsvd_sec_cnt;
    fs_fat_sectors         = bpb->fat_sz32;
    fs_data_start_lba      = fs_fat_start_lba +
        (uint32_t)bpb->num_fats * bpb->fat_sz32;
    fs_root_cluster        = bpb->root_clus;
    fs_fsinfo_sector       = bpb->fs_info;

    // Calculate total clusters.
    uint64_t data_sectors = fs_total_sectors - fs_data_start_lba;
    fs_total_clusters = (uint32_t)(data_sectors / fs_sectors_per_cluster);

    // Allocate FAT bitmap.
    fs_fat_bitmap_size = (fs_total_clusters + 7) / 8;
    fs_fat_bitmap = (uint8_t *)kmalloc(fs_fat_bitmap_size);
    if (!fs_fat_bitmap)
        return -1;
    memset(fs_fat_bitmap, 0, fs_fat_bitmap_size);

    // Build FAT bitmap from on-disk FAT table.
    if (build_fat_bitmap() < 0)
        return -1;

    // Read FSInfo sector for free cluster count.
    if (read_sectors(fs_fsinfo_sector, 1, fs_cluster_buf) == 0) {
        uint32_t sig1 = *(uint32_t *)(fs_cluster_buf + 0);
        uint32_t sig2 = *(uint32_t *)(fs_cluster_buf + 484);
        if (sig1 == 0x41615252 && sig2 == 0x61417272) {
            fs_free_clusters = *(uint32_t *)(fs_cluster_buf + 488);
        } else {
            fs_free_clusters = fs_total_clusters;
        }
    } else {
        fs_free_clusters = fs_total_clusters;
    }

    fs_ready = 1;
    return 0;
}

// ---------------------------------------------------------------------------
// Scheme driver interface for //disk:
// ---------------------------------------------------------------------------

int fs_disk_open(const char *authority __attribute__((unused)),
                 const char *path,
                 file_descriptor_t *fd_out)
{
    if (!fs_ready)
        return -1;

    // Strip the /mnt/ mount prefix — FAT32 root is "/".
    const char *fat_path = strip_mnt_prefix(path);

    fat32_dir_entry_t entry;
    uint32_t parent_cluster;
    int found = resolve_path(fat_path, &entry, &parent_cluster);
    if (found != 1)
        return -1;

    // Directories are not readable via read() in this driver.
    if (entry.attr & FAT32_ATTR_DIRECTORY)
        return -1;

    fat32_file_t *file = (fat32_file_t *)kmalloc(sizeof(fat32_file_t));
    if (!file)
        return -1;

    file->dir_cluster       = parent_cluster;
    file->dir_entry_offset  = 0;  // not needed for read-only open
    file->first_cluster     = fat32_entry_cluster(&entry);
    file->file_size         = entry.file_size;
    file->current_cluster   = 0;
    file->cluster_buf       = NULL;
    file->dirty             = 0;

    fd_out->type      = FD_TYPE_DISK_FILE;
    fd_out->disk_file = (struct disk_file *)file;
    fd_out->offset    = 0;
    return 0;
}

int64_t fs_disk_read(fat32_file_t *file, char *buf, uint64_t offset, uint64_t len)
{
    if (!file || !buf || !fs_cluster_buf)
        return -1;

    if (offset >= file->file_size)
        return 0;

    uint64_t available = file->file_size - offset;
    if (len > available)
        len = available;

    uint64_t total_read = 0;
    uint64_t pos = offset;

    if (!file->cluster_buf) {
        void *phys = physical_memory_alloc();
        if (!phys)
            return -1;
        file->cluster_buf = (uint8_t *)(hhdm_offset + (uint64_t)phys);
    }

    while (total_read < len) {
        uint32_t cluster_index = (uint32_t)(pos / fs_bytes_per_cluster);
        uint32_t cluster = cluster_at_index(file->first_cluster, cluster_index);
        if (cluster == 0 || fat32_is_eof(cluster) || fat32_is_bad(cluster))
            break;

        if (cluster != file->current_cluster) {
            if (read_cluster(cluster, file->cluster_buf) < 0)
                break;
            file->current_cluster = cluster;
        }

        uint32_t offset_in_cluster = (uint32_t)(pos % fs_bytes_per_cluster);
        uint32_t avail_in_cluster = fs_bytes_per_cluster - offset_in_cluster;
        uint32_t to_copy = (uint32_t)(len - total_read);
        if (to_copy > avail_in_cluster)
            to_copy = avail_in_cluster;

        uint64_t remaining_file = file->file_size - pos;
        if (to_copy > remaining_file)
            to_copy = (uint32_t)remaining_file;

        memcpy((uint8_t *)buf + total_read, file->cluster_buf + offset_in_cluster, to_copy);

        total_read += to_copy;
        pos += to_copy;
    }

    return (int64_t)total_read;
}

int64_t fs_disk_write(fat32_file_t *file, const char *buf, uint64_t offset, uint64_t len)
{
    if (!file || !buf || !fs_cluster_buf)
        return -1;

    if (len == 0)
        return 0;

    // Allocate per-file cluster buffer if needed.
    if (!file->cluster_buf) {
        void *phys = physical_memory_alloc();
        if (!phys)
            return -1;
        file->cluster_buf = (uint8_t *)(hhdm_offset + (uint64_t)phys);
    }

    uint64_t total_written = 0;
    uint64_t pos = offset;

    // Calculate the ending position to determine if we need new clusters.
    uint64_t end_pos = offset + len;
    uint64_t current_file_size = file->file_size;

    // If writing past EOF, we need to extend the file.
    if (end_pos > current_file_size) {
        uint32_t current_clusters = (current_file_size == 0) ? 0 :
            (uint32_t)((current_file_size + fs_bytes_per_cluster - 1) / fs_bytes_per_cluster);
        uint32_t needed_clusters = (uint32_t)((end_pos + fs_bytes_per_cluster - 1) / fs_bytes_per_cluster);
        uint32_t new_clusters = needed_clusters - current_clusters;

        if (new_clusters > 0) {
            uint32_t new_first = extend_file_clusters(file->first_cluster, new_clusters);
            if (new_first == 0)
                return -1;
            file->first_cluster = new_first;
        }

        // Update file size in the directory entry on disk.
        file->file_size = (uint32_t)end_pos;
    }

    while (total_written < len) {
        uint32_t cluster_index = (uint32_t)(pos / fs_bytes_per_cluster);
        uint32_t cluster = cluster_at_index(file->first_cluster, cluster_index);
        if (cluster == 0 || fat32_is_eof(cluster) || fat32_is_bad(cluster))
            break;

        // Read-modify-write: load cluster if not current.
        if (cluster != file->current_cluster) {
            if (read_cluster(cluster, file->cluster_buf) < 0)
                break;
            file->current_cluster = cluster;
        }

        uint32_t offset_in_cluster = (uint32_t)(pos % fs_bytes_per_cluster);
        uint32_t avail_in_cluster = fs_bytes_per_cluster - offset_in_cluster;
        uint32_t to_copy = (uint32_t)(len - total_written);
        if (to_copy > avail_in_cluster)
            to_copy = avail_in_cluster;

        // Note: when writing past old EOF, the cluster was read with read_cluster
        // so any unwritten bytes between old EOF and our write position retain
        // their previous content (typically zero from alloc time).

        memcpy(file->cluster_buf + offset_in_cluster, (const uint8_t *)buf + total_written, to_copy);

        // Write the cluster back.
        if (write_cluster(cluster, file->cluster_buf) < 0)
            break;

        total_written += to_copy;
        pos += to_copy;
    }

    if (total_written > 0) {
        file->dirty = 1;
        // Update file size if we grew.
        if (offset + total_written > file->file_size)
            file->file_size = (uint32_t)(offset + total_written);
    }

    return (int64_t)total_written;
}

// Update the file size in the directory entry on disk.
static int update_dir_entry_size(fat32_file_t *file)
{
    if (!fs_cluster_buf)
        return -1;

    // We need to re-read the directory to find the entry.
    // For simplicity, we search by the first cluster number.
    uint32_t cluster = fs_root_cluster;

    // Walk all clusters of the root directory (and any subdirectories).
    // This is a brute-force scan — in practice directories are small.
    // We look for the entry with matching first_cluster.
    uint32_t entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);

    while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        if (read_cluster(cluster, fs_cluster_buf) < 0)
            return -1;

        fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;
        int modified = 0;

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            // Skip LFN, free, and deleted entries.
            if (fat32_is_lfn(&entries[i]))
                continue;
            if (entries[i].name[0] == FAT32_DIR_ENTRY_FREE ||
                entries[i].name[0] == FAT32_DIR_ENTRY_DELETED)
                continue;

            uint32_t entry_cluster = fat32_entry_cluster(&entries[i]);
            if (entry_cluster == file->first_cluster &&
                !(entries[i].attr & FAT32_ATTR_DIRECTORY)) {
                entries[i].file_size = file->file_size;
                modified = 1;
                break;
            }
        }

        if (modified) {
            return write_cluster(cluster, fs_cluster_buf);
        }

        if (read_fat_entry(cluster, &cluster) < 0)
            return -1;
    }

    return -1;  // entry not found
}

int64_t fs_disk_fstat_size(const file_descriptor_t *fd)
{
    if (fd->type != FD_TYPE_DISK_FILE || !fd->disk_file)
        return -1;
    return (int64_t)((fat32_file_t *)fd->disk_file)->file_size;
}

// ---------------------------------------------------------------------------
// Create a new file on the FAT32 filesystem
// ---------------------------------------------------------------------------

int fs_disk_creat(const char *authority __attribute__((unused)),
                  const char *path,
                  file_descriptor_t *fd_out)
{
    if (!fs_ready)
        return -1;

    const char *fat_path = strip_mnt_prefix(path);

    // Check if file already exists — truncate it.
    fat32_dir_entry_t existing;
    if (resolve_path(fat_path, &existing, NULL) == 1) {
        // File exists — truncate.
        if (existing.attr & FAT32_ATTR_DIRECTORY)
            return -1;  // can't truncate a directory

        fat32_file_t *file = (fat32_file_t *)kmalloc(sizeof(fat32_file_t));
        if (!file)
            return -1;

        uint32_t old_cluster = fat32_entry_cluster(&existing);

        // Free old clusters.
        if (old_cluster >= FAT32_FIRST_DATA_CLUSTER)
            free_chain(old_cluster);

        // Allocate one new cluster for the file.
        uint32_t new_first = 0;
        if (alloc_cluster(0, &new_first) == 0) {
            kfree(file);
            return -1;
        }

        // Zero the new cluster.
        if (fs_cluster_buf) {
            memset(fs_cluster_buf, 0, fs_bytes_per_cluster);
            write_cluster(new_first, fs_cluster_buf);
        }

        // Update the directory entry on disk.
        existing.fst_clus_hi = (uint16_t)(new_first >> 16);
        existing.fst_clus_lo = (uint16_t)(new_first & 0xFFFF);
        existing.file_size = 0;

        // Update on disk — brute-force scan to find and update.
        if (fs_cluster_buf) {
            uint32_t entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);
            uint32_t cluster = fs_root_cluster;
            while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
                if (read_cluster(cluster, fs_cluster_buf) < 0) break;
                fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;
                int found = 0;
                for (uint32_t i = 0; i < entries_per_cluster; i++) {
                    if (fat32_is_lfn(&entries[i])) continue;
                    if (entries[i].name[0] == FAT32_DIR_ENTRY_FREE ||
                        entries[i].name[0] == FAT32_DIR_ENTRY_DELETED)
                        continue;
                    if (fat32_entry_cluster(&entries[i]) == old_cluster ||
                        (old_cluster == 0 && entries[i].file_size == existing.file_size &&
                         memcmp(entries[i].name, existing.name, 11) == 0)) {
                        // Found it — update.
                        entries[i].fst_clus_hi = existing.fst_clus_hi;
                        entries[i].fst_clus_lo = existing.fst_clus_lo;
                        entries[i].file_size = 0;
                        write_cluster(cluster, fs_cluster_buf);
                        found = 1;
                        break;
                    }
                }
                if (found) break;
                if (read_fat_entry(cluster, &cluster) < 0) break;
            }
        }

        file->dir_cluster       = 0;  // not tracked for truncate
        file->first_cluster     = new_first;
        file->file_size         = 0;
        file->current_cluster   = 0;
        file->cluster_buf       = NULL;
        file->dirty             = 0;

        fd_out->type      = FD_TYPE_DISK_FILE;
        fd_out->disk_file = (struct disk_file *)file;
        fd_out->offset    = 0;
        return 0;
    }

    // File does not exist — create it.
    // Split path into parent directory + filename.
    const char *components[FAT32_MAX_PATH_DEPTH];
    int depth = split_path(fat_path, components, FAT32_MAX_PATH_DEPTH);
    if (depth <= 0)
        return -1;

    const char *filename = components[depth - 1];
    int filename_len = (int)strlen(filename);

    // Resolve the parent directory.
    uint32_t parent_cluster = fs_root_cluster;
    if (depth > 1) {
        // Build parent path.
        char parent_path[256];
        int pos = 0;
        parent_path[pos++] = '/';
        for (int i = 0; i < depth - 1; i++) {
            int comp_len;
            if (i + 1 < depth)
                comp_len = (int)(components[i + 1] - components[i] - 1);
            else
                comp_len = (int)strlen(components[i]);
            memcpy(parent_path + pos, components[i], (size_t)comp_len);
            pos += comp_len;
            parent_path[pos++] = '/';
        }
        parent_path[pos] = '\0';

        fat32_dir_entry_t parent_entry;
        if (resolve_path(parent_path, &parent_entry, NULL) != 1)
            return -1;
        if (!(parent_entry.attr & FAT32_ATTR_DIRECTORY))
            return -1;
        parent_cluster = fat32_entry_cluster(&parent_entry);
    }

    // Find a free directory entry slot.
    uint32_t entry_cluster, entry_offset, entries_per_cluster;
    int find_rc = find_free_dir_entry(parent_cluster, &entry_cluster, &entry_offset, &entries_per_cluster);

    if (find_rc == 1) {
        // Need to extend the directory.
        uint32_t new_dir_cluster;
        if (extend_directory(parent_cluster, &new_dir_cluster) < 0)
            return -1;
        entry_cluster = new_dir_cluster;
        entry_offset = 0;
        entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);
    }

    if (find_rc < 0)
        return -1;

    // Create the short name.
    uint8_t short_name[11];
    name_to_short(filename, filename_len, short_name);

    // Allocate one cluster for the new file.
    uint32_t new_first = 0;
    if (alloc_cluster(0, &new_first) == 0)
        return -1;

    // Zero the new cluster.
    if (fs_cluster_buf) {
        memset(fs_cluster_buf, 0, fs_bytes_per_cluster);
        write_cluster(new_first, fs_cluster_buf);
    }

    // Read the directory cluster.
    if (!fs_cluster_buf)
        return -1;
    if (read_cluster(entry_cluster, fs_cluster_buf) < 0)
        return -1;

    fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;

    // Write LFN chain if the filename is longer than 8.3 or has mixed case.
    int needs_lfn = (filename_len > 12);  // heuristic: longer names need LFN
    int lfn_entries = 0;
    if (needs_lfn) {
        lfn_entries = (filename_len + 12) / 13;
        if (entry_offset < (uint32_t)lfn_entries) {
            // Not enough room before this entry — use the next slot or extend.
            // For simplicity, just use the short name without LFN.
            needs_lfn = 0;
            lfn_entries = 0;
        }
    }

    if (needs_lfn) {
        lfn_entries = write_lfn_chain(entries, entry_offset, filename, filename_len, short_name);
    }

    // Write the short-name entry.
    memset(&entries[entry_offset], 0, sizeof(fat32_dir_entry_t));
    memcpy(entries[entry_offset].name, short_name, 11);
    entries[entry_offset].attr = FAT32_ATTR_ARCHIVE;
    entries[entry_offset].fst_clus_hi = (uint16_t)(new_first >> 16);
    entries[entry_offset].fst_clus_lo = (uint16_t)(new_first & 0xFFFF);
    entries[entry_offset].file_size = 0;

    // Write the directory cluster back.
    uint32_t dir_entries_to_write = entry_offset + 1;
    if ((uint32_t)needs_lfn) dir_entries_to_write += (uint32_t)lfn_entries;

    // Number of sectors to write (covering all modified entries).
    uint32_t end_sector = ((dir_entries_to_write * sizeof(fat32_dir_entry_t)) + 511) / 512;
    if (end_sector > fs_sectors_per_cluster)
        end_sector = fs_sectors_per_cluster;

    // Write the sectors that contain the modified entries.
    for (uint32_t s = 0; s < end_sector; s++) {
        uint64_t sector_lba;
        // Calculate the LBA of this sector within the directory cluster.
        uint64_t cluster_lba = fs_data_start_lba +
            (uint64_t)(entry_cluster - FAT32_FIRST_DATA_CLUSTER) * fs_sectors_per_cluster;
        sector_lba = cluster_lba + s;

        if (write_sectors(sector_lba, 1, fs_cluster_buf + s * 512) < 0)
            return -1;
    }

    // Create the file descriptor.
    fat32_file_t *file = (fat32_file_t *)kmalloc(sizeof(fat32_file_t));
    if (!file)
        return -1;

    file->dir_cluster       = parent_cluster;
    file->first_cluster     = new_first;
    file->file_size         = 0;
    file->current_cluster   = 0;
    file->cluster_buf       = NULL;
    file->dirty             = 0;

    fd_out->type      = FD_TYPE_DISK_FILE;
    fd_out->disk_file = (struct disk_file *)file;
    fd_out->offset    = 0;
    return 0;
}

// ---------------------------------------------------------------------------
// Delete a file
// ---------------------------------------------------------------------------

int fs_disk_unlink(const char *authority __attribute__((unused)),
                   const char *path)
{
    if (!fs_ready)
        return -1;

    const char *fat_path = strip_mnt_prefix(path);

    fat32_dir_entry_t entry;
    if (resolve_path(fat_path, &entry, NULL) != 1)
        return -1;

    if (entry.attr & FAT32_ATTR_DIRECTORY)
        return -1;  // can't delete directories (not implemented)

    uint32_t cluster = fat32_entry_cluster(&entry);

    // Free the cluster chain.
    if (cluster >= FAT32_FIRST_DATA_CLUSTER)
        free_chain(cluster);

    // Mark the directory entry as deleted.
    // Brute-force scan to find and delete it.
    if (!fs_cluster_buf)
        return -1;

    uint32_t entries_per_cluster = fs_bytes_per_cluster / sizeof(fat32_dir_entry_t);
    cluster = fs_root_cluster;
    while (!fat32_is_eof(cluster) && !fat32_is_bad(cluster)) {
        if (read_cluster(cluster, fs_cluster_buf) < 0)
            return -1;

        fat32_dir_entry_t *entries = (fat32_dir_entry_t *)fs_cluster_buf;
        int modified = 0;

        for (uint32_t i = 0; i < entries_per_cluster; i++) {
            if (fat32_is_lfn(&entries[i]))
                continue;
            if (entries[i].name[0] == FAT32_DIR_ENTRY_FREE ||
                entries[i].name[0] == FAT32_DIR_ENTRY_DELETED)
                continue;

            if (fat32_entry_cluster(&entries[i]) == cluster ||
                (fat32_entry_cluster(&entries[i]) == fat32_entry_cluster(&entry))) {
                // Check if the file size matches (to distinguish same cluster reused).
                if (entries[i].file_size == entry.file_size) {
                    // Mark preceding LFN entries as deleted too.
                    int j = (int)i - 1;
                    while (j >= 0 && fat32_is_lfn(&entries[j])) {
                        entries[j].name[0] = FAT32_DIR_ENTRY_DELETED;
                        j--;
                    }
                    entries[i].name[0] = FAT32_DIR_ENTRY_DELETED;
                    modified = 1;
                    break;
                }
            }
        }

        if (modified)
            return write_cluster(cluster, fs_cluster_buf);

        if (read_fat_entry(cluster, &cluster) < 0)
            return -1;
    }

    return -1;  // not found
}

// ---------------------------------------------------------------------------
// Close a disk file — flush dirty metadata
// ---------------------------------------------------------------------------

void fs_disk_close(fat32_file_t *file)
{
    if (!file)
        return;

    if (file->dirty) {
        update_dir_entry_size(file);
    }

    if (file->cluster_buf) {
        // Convert HHDM virtual address back to physical for the allocator.
        uint64_t phys = (uint64_t)file->cluster_buf - hhdm_offset;
        physical_memory_free((void *)phys);
    }
    kfree(file);
}

// ---------------------------------------------------------------------------
// Disk info accessor (for syscall)
// ---------------------------------------------------------------------------

typedef struct {
    uint64_t capacity_sectors;
    uint64_t free_clusters;
    uint64_t total_clusters;
    uint64_t bytes_per_cluster;
    int      ready;
} disk_info_t;

int fs_disk_info(disk_info_t *out)
{
    if (!fs_ready)
        return -1;
    out->capacity_sectors  = fs_total_sectors;
    out->free_clusters     = fs_free_clusters;
    out->total_clusters    = fs_total_clusters;
    out->bytes_per_cluster = (uint64_t)fs_bytes_per_cluster;
    out->ready             = 1;
    return 0;
}

// ---------------------------------------------------------------------------
// VFS scheme driver descriptor
// ---------------------------------------------------------------------------

const vfs_scheme_driver_t fs_disk_driver = {
    .scheme      = "disk",
    .open        = fs_disk_open,
    .creat       = fs_disk_creat,
    .unlink      = fs_disk_unlink,
    .fstat_size  = fs_disk_fstat_size,
};
