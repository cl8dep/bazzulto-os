#include "../library/systemcall.h"
#include <string.h>

int main(int argc, const char *argv[])
{
    if (argc < 2) {
        write(1, "usage: touch <file...>\r\n", 24);
        return 1;
    }

    for (int i = 1; i < argc; i++) {
        int fd = creat(argv[i]);
        if (fd < 0) {
            write(1, argv[i], strlen(argv[i]));
            write(1, ": cannot create\r\n", 17);
            continue;
        }
        close(fd);
    }
    return 0;
}
