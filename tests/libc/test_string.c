/* test_string.c — memcpy, memmove, memset, strcmp, strcpy, strcat, strstr, strtok_r */
#include <stdio.h>
#include <string.h>

int main(void) {
    int pass = 1;
    char buf[128];

    /* memcpy */
    memcpy(buf, "hello world", 12);
    if (strcmp(buf, "hello world") != 0) { puts("FAIL memcpy"); pass = 0; }

    /* memmove — overlapping regions */
    memmove(buf + 2, buf, 12);
    if (memcmp(buf + 2, "hello world", 12) != 0) { puts("FAIL memmove"); pass = 0; }

    /* memset */
    memset(buf, 'X', 10);
    buf[10] = '\0';
    if (strcmp(buf, "XXXXXXXXXX") != 0) { puts("FAIL memset"); pass = 0; }

    /* strcpy + strlen */
    strcpy(buf, "abc");
    if (strlen(buf) != 3) { puts("FAIL strlen"); pass = 0; }

    /* strcat */
    strcat(buf, "def");
    if (strcmp(buf, "abcdef") != 0) { puts("FAIL strcat"); pass = 0; }

    /* strncmp */
    if (strncmp("abcdef", "abcxyz", 3) != 0) { puts("FAIL strncmp"); pass = 0; }
    if (strncmp("abcdef", "abcxyz", 4) == 0) { puts("FAIL strncmp 2"); pass = 0; }

    /* strstr */
    if (strstr("hello world", "world") == NULL) { puts("FAIL strstr"); pass = 0; }
    if (strstr("hello world", "xyz") != NULL) { puts("FAIL strstr null"); pass = 0; }

    /* strtok_r */
    char csv[] = "one,two,,three";
    char *saveptr;
    char *tok = strtok_r(csv, ",", &saveptr);
    if (!tok || strcmp(tok, "one") != 0) { puts("FAIL strtok_r 1"); pass = 0; }
    tok = strtok_r(NULL, ",", &saveptr);
    if (!tok || strcmp(tok, "two") != 0) { puts("FAIL strtok_r 2"); pass = 0; }
    tok = strtok_r(NULL, ",", &saveptr);
    if (!tok || strcmp(tok, "three") != 0) { puts("FAIL strtok_r 3"); pass = 0; }

    if (pass) puts("PASS test_string");
    return pass ? 0 : 1;
}
