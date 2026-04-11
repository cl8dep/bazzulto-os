#include "../library/systemcall.h"
#include <stdio.h>
#include <string.h>

#define BUF_SIZE 512

static char buf[BUF_SIZE];

int main(void)
{
    write(1, "PID   STATUS\r\n", 14);

    // Iterate PIDs 1..255 trying //proc:<pid>/status.
    for (int pid = 1; pid <= 255; pid++) {
        char path[32];
        // Build //proc:<pid>/status
        int pos = 0;
        const char *prefix = "//proc:";
        while (*prefix) path[pos++] = *prefix++;

        // Decimal PID
        char num[8];
        int nlen = 0;
        int tmp = pid;
        if (tmp == 0) { num[nlen++] = '0'; }
        else {
            char rev[8]; int rlen = 0;
            while (tmp > 0) { rev[rlen++] = (char)('0' + tmp % 10); tmp /= 10; }
            for (int i = rlen - 1; i >= 0; i--) num[nlen++] = rev[i];
        }
        for (int i = 0; i < nlen; i++) path[pos++] = num[i];

        const char *suffix = "/status";
        while (*suffix) path[pos++] = *suffix++;
        path[pos] = '\0';

        int fd = open(path);
        if (fd < 0)
            continue;

        int64_t n = read(fd, buf, BUF_SIZE - 1);
        close(fd);
        if (n <= 0)
            continue;

        buf[n] = '\0';

        // Print the status block.
        write(1, buf, (size_t)n);
        write(1, "\r\n", 2);
    }
    return 0;
}
