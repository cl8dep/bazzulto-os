#include "../library/systemcall.h"
#include <string.h>

#define BUF_SIZE 4096

static char read_buf[BUF_SIZE];

static char hex_char(int nibble)
{
    if (nibble < 10) return (char)('0' + nibble);
    return (char)('a' + nibble - 10);
}

static void dump_fd(int fd)
{
    uint64_t offset = 0;
    int64_t n;
    unsigned char row[16];
    char line[80];

    while ((n = read(fd, read_buf, BUF_SIZE)) > 0) {
        int64_t i = 0;
        while (i < n) {
            int row_len = (int)(n - i);
            if (row_len > 16) row_len = 16;
            for (int j = 0; j < row_len; j++)
                row[j] = (unsigned char)read_buf[i + j];

            // Format: XXXXXXXX  xx xx xx xx xx xx xx xx  xx xx xx xx xx xx xx xx  |................|
            int pos = 0;

            // Address
            for (int shift = 28; shift >= 0; shift -= 4)
                line[pos++] = hex_char((int)((offset >> shift) & 0xF));
            line[pos++] = ' ';
            line[pos++] = ' ';

            // Hex bytes (two groups of 8)
            for (int j = 0; j < 16; j++) {
                if (j == 8) line[pos++] = ' ';
                if (j < row_len) {
                    line[pos++] = hex_char(row[j] >> 4);
                    line[pos++] = hex_char(row[j] & 0xF);
                } else {
                    line[pos++] = ' ';
                    line[pos++] = ' ';
                }
                line[pos++] = ' ';
            }

            // ASCII
            line[pos++] = ' ';
            line[pos++] = '|';
            for (int j = 0; j < 16; j++) {
                if (j < row_len) {
                    unsigned char c = row[j];
                    line[pos++] = (c >= 0x20 && c < 0x7F) ? (char)c : '.';
                } else {
                    line[pos++] = ' ';
                }
            }
            line[pos++] = '|';
            line[pos++] = '\r';
            line[pos++] = '\n';
            write(1, line, (size_t)pos);

            offset += (uint64_t)row_len;
            i += row_len;
        }
    }
}

int main(int argc, const char *argv[])
{
    if (argc <= 1) {
        dump_fd(0);
        return 0;
    }

    int fd = open(argv[1]);
    if (fd < 0) {
        write(1, argv[1], strlen(argv[1]));
        write(1, ": no such file\r\n", 16);
        return 1;
    }
    dump_fd(fd);
    close(fd);
    return 0;
}
