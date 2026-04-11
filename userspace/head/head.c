#include "../library/systemcall.h"
#include <string.h>

#define BUF_SIZE  4096
#define LINE_SIZE 1024

static char read_buf[BUF_SIZE];
static char line_buf[LINE_SIZE];

static void head_fd(int fd, int max_lines)
{
    int line_pos = 0;
    int lines_printed = 0;
    int64_t n;
    while (lines_printed < max_lines && (n = read(fd, read_buf, BUF_SIZE)) > 0) {
        for (int64_t i = 0; i < n && lines_printed < max_lines; i++) {
            char ch = read_buf[i];
            if (ch == '\n') {
                // Newline terminates a line (handles both LF and CRLF).
                line_buf[line_pos] = '\0';
                write(1, line_buf, (size_t)line_pos);
                write(1, "\r\n", 2);
                line_pos = 0;
                lines_printed++;
            } else if (ch == '\r') {
                // Carriage return: skip (the \n that follows will flush).
            } else if (line_pos < LINE_SIZE - 1) {
                line_buf[line_pos++] = ch;
            }
        }
    }
    // Flush trailing partial line.
    if (line_pos > 0 && lines_printed < max_lines) {
        write(1, line_buf, (size_t)line_pos);
        write(1, "\r\n", 2);
    }
}

int main(int argc, const char *argv[])
{
    int max_lines = 10;
    int file_start = 1;

    if (argc >= 3 && strcmp(argv[1], "-n") == 0) {
        const char *p = argv[2];
        max_lines = 0;
        while (*p >= '0' && *p <= '9')
            max_lines = max_lines * 10 + (*p++ - '0');
        if (max_lines <= 0) max_lines = 10;
        file_start = 3;
    }

    if (file_start >= argc) {
        head_fd(0, max_lines);
    } else {
        int fd = open(argv[file_start]);
        if (fd < 0) {
            write(1, argv[file_start], strlen(argv[file_start]));
            write(1, ": no such file\r\n", 16);
            return 1;
        }
        head_fd(fd, max_lines);
        close(fd);
    }
    return 0;
}
