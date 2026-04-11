// df — display disk free space
//
// Usage: df
// Shows total, used, and free space on the mounted FAT32 disk.

#include "../library/systemcall.h"
#include "../libc/stdio.h"

static void print_human(uint64_t bytes)
{
    if (bytes >= 1073741824ULL) {
        // GB
        uint64_t gb = bytes / 1073741824ULL;
        uint64_t frac = (bytes % 1073741824ULL) * 10 / 1073741824ULL;
        printf("%lu.%lu GB", (unsigned long)gb, (unsigned long)frac);
    } else if (bytes >= 1048576ULL) {
        // MB
        uint64_t mb = bytes / 1048576ULL;
        uint64_t frac = (bytes % 1048576ULL) * 10 / 1048576ULL;
        printf("%lu.%lu MB", (unsigned long)mb, (unsigned long)frac);
    } else if (bytes >= 1024ULL) {
        // KB
        uint64_t kb = bytes / 1024ULL;
        uint64_t frac = (bytes % 1024ULL) * 10 / 1024ULL;
        printf("%lu.%lu KB", (unsigned long)kb, (unsigned long)frac);
    } else {
        printf("%lu B", (unsigned long)bytes);
    }
}

int main(void)
{
    struct disk_info info;

    if (disk_info(&info) < 0) {
        printf("df: no disk filesystem available\r\n");
        return 1;
    }

    uint64_t total_bytes = info.total_clusters * info.bytes_per_cluster;
    uint64_t free_bytes  = info.free_clusters * info.bytes_per_cluster;
    uint64_t used_bytes  = total_bytes - free_bytes;

    printf("Filesystem     Size    Used    Free  Use%%\r\n");
    printf("/mnt/          ");
    print_human(total_bytes);
    printf("  ");
    print_human(used_bytes);
    printf("  ");
    print_human(free_bytes);

    if (total_bytes > 0) {
        uint64_t pct = used_bytes * 100 / total_bytes;
        printf("  %lu%%", (unsigned long)pct);
    } else {
        printf("   0%%");
    }
    printf("\r\n");

    // Also show raw details.
    printf("\r\n");
    printf("Disk capacity:   %lu sectors (%lu bytes)\r\n",
           (unsigned long)info.capacity_sectors, (unsigned long)(info.capacity_sectors * 512));
    printf("Cluster size:    %lu bytes\r\n", (unsigned long)info.bytes_per_cluster);
    printf("Total clusters:  %lu\r\n", (unsigned long)info.total_clusters);
    printf("Free clusters:   %lu\r\n", (unsigned long)info.free_clusters);
    printf("Used clusters:   %lu\r\n", (unsigned long)(info.total_clusters - info.free_clusters));

    return 0;
}
