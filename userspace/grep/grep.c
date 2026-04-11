#include "../library/systemcall.h"
#include <string.h>
#include <stdio.h>

#define BUF_SIZE  4096
#define LINE_SIZE 1024

static char read_buf[BUF_SIZE];
static char line_buf[LINE_SIZE];

static void grep_fd(int fd, const char *pattern)
{
    int line_pos = 0;
    int64_t n;
    while ((n = read(fd, read_buf, BUF_SIZE)) > 0) {
        for (int64_t i = 0; i < n; i++) {
            char ch = read_buf[i];
            if (ch == '\n') {
                line_buf[line_pos] = '\0';
                if (strstr(line_buf, pattern)) {
                    write(1, line_buf, (size_t)line_pos);
                    write(1, "\r\n", 2);
                }
                line_pos = 0;
            } else if (ch == '\r') {
                // skip CR (CRLF lines: \n flushes, \r is ignored)
            } else if (line_pos < LINE_SIZE - 1) {
                line_buf[line_pos++] = ch;
            }
        }
    }
    // Flush any remaining partial line (no trailing newline in input).
    if (line_pos > 0) {
        line_buf[line_pos] = '\0';
        if (strstr(line_buf, pattern)) {
            write(1, line_buf, (size_t)line_pos);
            write(1, "\r\n", 2);
        }
    }
    line_pos = 0;
}

int main(int argc, const char *argv[])
{
    if (argc < 2) {
        write(1, "usage: grep <pattern> [file]\r\n", 29);
        return 1;
    }

    const char *pattern = argv[1];

    if (argc == 2) {
        grep_fd(0, pattern);
    } else {
        int fd = open(argv[2]);
        if (fd < 0) {
            write(1, argv[2], strlen(argv[2]));
            write(1, ": no such file\r\n", 16);
            return 1;
        }
        grep_fd(fd, pattern);
        close(fd);
    }
    return 0;
}
