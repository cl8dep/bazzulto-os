#include "../library/systemcall.h"
#include <string.h>

int main(int argc, const char *argv[])
{
    if (argc < 2) {
        write(1, "usage: sleep <seconds>\r\n", 24);
        return 1;
    }

    // Parse decimal seconds (e.g. "2" or "0.5").
    const char *s = argv[1];
    long long whole = 0;
    long long frac_ns = 0;

    while (*s >= '0' && *s <= '9')
        whole = whole * 10 + (*s++ - '0');

    if (*s == '.') {
        s++;
        long long multiplier = 100000000LL;  // start at 0.1s in ns
        while (*s >= '0' && *s <= '9' && multiplier > 0) {
            frac_ns += (*s++ - '0') * multiplier;
            multiplier /= 10;
        }
    }

    struct timespec req;
    req.tv_sec  = whole;
    req.tv_nsec = frac_ns;
    nanosleep(&req, (struct timespec *)0);
    return 0;
}
