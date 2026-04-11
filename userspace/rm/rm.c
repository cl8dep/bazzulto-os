#include "../library/systemcall.h"
#include <string.h>

int main(int argc, const char *argv[])
{
    if (argc < 2) {
        write(1, "usage: rm <file...>\r\n", 21);
        return 1;
    }

    int errors = 0;
    for (int i = 1; i < argc; i++) {
        if (unlink(argv[i]) < 0) {
            write(1, argv[i], strlen(argv[i]));
            write(1, ": no such file\r\n", 16);
            errors++;
        }
    }
    return errors ? 1 : 0;
}
