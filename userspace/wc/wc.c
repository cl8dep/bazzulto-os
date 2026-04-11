#include "../library/systemcall.h"
#include <stdio.h>
#include <string.h>

#define BUF_SIZE 4096

static char buf[BUF_SIZE];

typedef struct {
    long long lines;
    long long words;
    long long bytes;
} counts_t;

static counts_t count_fd(int fd)
{
    counts_t c = {0, 0, 0};
    int in_word = 0;
    int64_t n;
    while ((n = read(fd, buf, BUF_SIZE)) > 0) {
        c.bytes += n;
        for (int64_t i = 0; i < n; i++) {
            char ch = buf[i];
            if (ch == '\n') c.lines++;
            int space = (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r');
            if (!space && !in_word) { c.words++; in_word = 1; }
            else if (space)         { in_word = 0; }
        }
    }
    return c;
}

int main(int argc, const char *argv[])
{
    // Parse flags: -l (lines), -w (words), -c (bytes). Default: all three.
    int show_lines = 0, show_words = 0, show_bytes = 0;
    int file_start = 1;

    for (int i = 1; i < argc; i++) {
        if (argv[i][0] == '-' && argv[i][1] != '\0') {
            for (int j = 1; argv[i][j]; j++) {
                if (argv[i][j] == 'l') show_lines = 1;
                else if (argv[i][j] == 'w') show_words = 1;
                else if (argv[i][j] == 'c') show_bytes = 1;
            }
            file_start = i + 1;
        } else {
            break;
        }
    }
    if (!show_lines && !show_words && !show_bytes)
        show_lines = show_words = show_bytes = 1;

    int fd;
    if (file_start >= argc) {
        fd = 0;
    } else {
        fd = open(argv[file_start]);
        if (fd < 0) {
            write(1, argv[file_start], strlen(argv[file_start]));
            write(1, ": no such file\r\n", 16);
            return 1;
        }
    }

    counts_t c = count_fd(fd);
    if (fd != 0) close(fd);

    char out[128];
    int pos = 0;
    if (show_lines) {
        int n = sprintf(out + pos, "%lld", c.lines);
        pos += n;
        out[pos++] = ' ';
    }
    if (show_words) {
        int n = sprintf(out + pos, "%lld", c.words);
        pos += n;
        out[pos++] = ' ';
    }
    if (show_bytes) {
        int n = sprintf(out + pos, "%lld", c.bytes);
        pos += n;
        out[pos++] = ' ';
    }
    out[pos++] = '\r';
    out[pos++] = '\n';
    write(1, out, (size_t)pos);
    return 0;
}
