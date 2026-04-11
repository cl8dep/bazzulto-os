#include "../library/systemcall.h"
#include <string.h>

#define BUF_SIZE 4096

static char buf[BUF_SIZE];

int main(int argc, const char *argv[])
{
    if (argc < 3) {
        write(1, "usage: cp <src> <dst>\r\n", 22);
        return 1;
    }

    int src_fd = open(argv[1]);
    if (src_fd < 0) {
        write(1, argv[1], strlen(argv[1]));
        write(1, ": no such file\r\n", 16);
        return 1;
    }

    int dst_fd = creat(argv[2]);
    if (dst_fd < 0) {
        close(src_fd);
        write(1, argv[2], strlen(argv[2]));
        write(1, ": cannot create\r\n", 17);
        return 1;
    }

    int64_t n;
    while ((n = read(src_fd, buf, BUF_SIZE)) > 0)
        write(dst_fd, buf, (size_t)n);

    close(src_fd);
    close(dst_fd);
    return 0;
}
