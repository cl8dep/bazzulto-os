#include "../library/systemcall.h"
#include <string.h>

#define BUF_SIZE 4096

static char buf[BUF_SIZE];

int main(int argc, const char *argv[])
{
    int file_fd = -1;

    if (argc >= 2) {
        file_fd = creat(argv[1]);
        if (file_fd < 0) {
            write(1, argv[1], strlen(argv[1]));
            write(1, ": cannot create\r\n", 17);
            return 1;
        }
    }

    int64_t n;
    while ((n = read(0, buf, BUF_SIZE)) > 0) {
        write(1, buf, (size_t)n);
        if (file_fd >= 0)
            write(file_fd, buf, (size_t)n);
    }

    if (file_fd >= 0)
        close(file_fd);
    return 0;
}
