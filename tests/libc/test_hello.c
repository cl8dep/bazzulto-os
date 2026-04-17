#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    printf("Hello from musl libc on Bazzulto OS!\n");
    printf("argc = %d\n", argc);

    char buf[64];
    snprintf(buf, sizeof(buf), "snprintf works: %d + %d = %d", 2, 3, 2+3);
    puts(buf);

    char *p = malloc(128);
    if (p) {
        strcpy(p, "malloc + strcpy works!");
        puts(p);
        free(p);
    }

    printf("All libc tests passed.\n");
    return 0;
}
