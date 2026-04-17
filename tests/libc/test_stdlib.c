/* test_stdlib.c — malloc, calloc, realloc, free, atoi, strtol, qsort */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int cmp_int(const void *a, const void *b) {
    return *(const int *)a - *(const int *)b;
}

int main(void) {
    int pass = 1;

    /* malloc + free */
    char *p = malloc(256);
    if (!p) { puts("FAIL malloc"); return 1; }
    memset(p, 'A', 256);
    if (p[255] != 'A') { puts("FAIL memset"); pass = 0; }
    free(p);

    /* calloc — zero-initialized */
    int *arr = calloc(64, sizeof(int));
    if (!arr) { puts("FAIL calloc"); return 1; }
    for (int i = 0; i < 64; i++) {
        if (arr[i] != 0) { puts("FAIL calloc zero"); pass = 0; break; }
    }
    free(arr);

    /* realloc */
    p = malloc(16);
    strcpy(p, "hello");
    p = realloc(p, 256);
    if (!p || strcmp(p, "hello") != 0) { puts("FAIL realloc"); pass = 0; }
    free(p);

    /* atoi / strtol */
    if (atoi("42") != 42) { puts("FAIL atoi"); pass = 0; }
    if (atoi("-7") != -7) { puts("FAIL atoi neg"); pass = 0; }
    if (strtol("0xff", NULL, 16) != 255) { puts("FAIL strtol hex"); pass = 0; }
    if (strtol("077", NULL, 0) != 63) { puts("FAIL strtol oct"); pass = 0; }

    /* qsort */
    int data[] = {5, 3, 8, 1, 9, 2, 7, 4, 6, 0};
    qsort(data, 10, sizeof(int), cmp_int);
    for (int i = 0; i < 10; i++) {
        if (data[i] != i) { puts("FAIL qsort"); pass = 0; break; }
    }

    if (pass) puts("PASS test_stdlib");
    return pass ? 0 : 1;
}
