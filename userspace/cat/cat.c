#include "../library/systemcall.h"
#include <string.h>

#define BUF_SIZE 4096

static char buf[BUF_SIZE];

static void cat_fd(int fd)
{
    int64_t n;
    while ((n = read(fd, buf, BUF_SIZE)) > 0)
        write(1, buf, (size_t)n);
}

int main(int argc, const char *argv[])
{
    if (argc <= 1) {
        // No arguments: read from stdin.
        cat_fd(0);
        return 0;
    }

    for (int i = 1; i < argc; i++) {
        int fd = open(argv[i]);
        if (fd < 0) {
            write(1, argv[i], strlen(argv[i]));
            write(1, ": no such file\r\n", 16);
            continue;
        }
        cat_fd(fd);
        close(fd);
    }
    return 0;
}
