// mount — display mounted filesystems
//
// Usage: mount
// Shows the static mount table and disk status.

#include "../library/systemcall.h"
#include "../libc/stdio.h"

int main(void)
{
    struct disk_info info;
    int disk_ready = (disk_info(&info) == 0 && info.ready);

    printf("Filesystem  Type    Mount point  Status\r\n");
    printf("----------  ------  -----------  ----------\r\n");
    printf("//ram:      ramfs   /tmp/        ready\r\n");
    printf("//ram:      ramfs   /run/        ready\r\n");
    printf("//ram:      ramfs   /var/        ready\r\n");
    printf("//ram:      ramfs   /dev/        ready\r\n");
    printf("//proc:     procfs  /proc/       ready\r\n");
    printf("//system:   ramfs   /bin/        ready\r\n");
    printf("//system:   ramfs   /lib/        ready\r\n");
    printf("//system:   ramfs   /etc/        ready\r\n");

    if (disk_ready) {
        uint64_t total_mb = (info.total_clusters * info.bytes_per_cluster) / (1024 * 1024);
        uint64_t free_mb  = (info.free_clusters * info.bytes_per_cluster) / (1024 * 1024);
        printf("//disk:     FAT32   /mnt/        ready (%luMB free of %luMB)\r\n",
               (unsigned long)free_mb, (unsigned long)total_mb);
    } else {
        printf("//disk:     FAT32   /mnt/        not available\r\n");
    }

    return 0;
}
