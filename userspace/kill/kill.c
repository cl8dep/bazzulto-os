#include "../library/systemcall.h"
#include <stdio.h>
#include <string.h>

static int parse_int(const char *s)
{
    int n = 0;
    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    while (*s >= '0' && *s <= '9')
        n = n * 10 + (*s++ - '0');
    return neg ? -n : n;
}

int main(int argc, const char *argv[])
{
    if (argc < 2) {
        write(1, "usage: kill <pid> [signal]\r\n", 27);
        return 1;
    }

    int pid = parse_int(argv[1]);
    int signum = SIGTERM;  // default

    if (argc >= 3)
        signum = parse_int(argv[2]);

    int result = kill(pid, signum);
    if (result < 0) {
        write(1, "kill: failed\r\n", 14);
        return 1;
    }
    return 0;
}
