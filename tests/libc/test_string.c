// Tests for userspace/libc/string.c
// Compile and run on the host: cc -o /tmp/test_string tests/libc/test_string.c userspace/libc/string.c && /tmp/test_string

#include "../../userspace/libc/string.h"
#include <stdio.h>
#include <stdint.h>

static int passed = 0;
static int failed = 0;

#define TEST(desc, expr) do {                               \
    if (expr) {                                             \
        printf("  PASS  %s\n", desc);                       \
        passed++;                                           \
    } else {                                                \
        printf("  FAIL  %s  (line %d)\n", desc, __LINE__); \
        failed++;                                           \
    }                                                       \
} while (0)

// ---------------------------------------------------------------------------
// memset / memcpy / memmove / memcmp / memchr
// ---------------------------------------------------------------------------

static void test_mem(void)
{
    printf("\nmemset\n");
    char buf[8];
    memset(buf, 0xAB, 8);
    TEST("fills all bytes",     (unsigned char)buf[0] == 0xAB && (unsigned char)buf[7] == 0xAB);
    memset(buf, 0, 8);
    TEST("fills with zero",     buf[0] == 0 && buf[7] == 0);
    TEST("returns dst pointer", memset(buf, 1, 1) == buf);

    printf("\nmemcpy\n");
    char src[4] = {1, 2, 3, 4};
    char dst[4] = {0};
    memcpy(dst, src, 4);
    TEST("copies all bytes",    dst[0]==1 && dst[1]==2 && dst[2]==3 && dst[3]==4);
    TEST("returns dst pointer", memcpy(dst, src, 1) == dst);

    printf("\nmemmove\n");
    char overlap[8] = {1,2,3,4,5,6,7,8};
    memmove(overlap + 2, overlap, 4);   // forward overlap
    TEST("forward overlap",     overlap[2]==1 && overlap[3]==2 && overlap[4]==3 && overlap[5]==4);
    char back[8] = {1,2,3,4,5,6,7,8};
    memmove(back, back + 2, 4);         // backward overlap
    TEST("backward overlap",    back[0]==3 && back[1]==4 && back[2]==5 && back[3]==6);

    printf("\nmemcmp\n");
    TEST("equal",               memcmp("abc", "abc", 3) == 0);
    TEST("less",                memcmp("abc", "abd", 3)  < 0);
    TEST("greater",             memcmp("abd", "abc", 3)  > 0);
    TEST("zero length",         memcmp("abc", "xyz", 0) == 0);

    printf("\nmemchr\n");
    const char *h = "hello";
    TEST("found",               memchr(h, 'l', 5) == h + 2);
    TEST("not found",           memchr(h, 'z', 5) == (void *)0);
    TEST("zero length",         memchr(h, 'h', 0) == (void *)0);
}

// ---------------------------------------------------------------------------
// strlen
// ---------------------------------------------------------------------------

static void test_strlen(void)
{
    printf("\nstrlen\n");
    TEST("empty",   strlen("") == 0);
    TEST("hello",   strlen("hello") == 5);
    TEST("single",  strlen("x") == 1);
}

// ---------------------------------------------------------------------------
// strcmp / strncmp
// ---------------------------------------------------------------------------

static void test_strcmp(void)
{
    printf("\nstrcmp / strncmp\n");
    TEST("equal",            strcmp("abc", "abc") == 0);
    TEST("less",             strcmp("abc", "abd")  < 0);
    TEST("greater",          strcmp("abd", "abc")  > 0);
    TEST("prefix shorter",   strcmp("ab",  "abc")  < 0);
    TEST("prefix longer",    strcmp("abc", "ab")   > 0);

    TEST("strncmp equal n",  strncmp("abcX", "abcY", 3) == 0);
    TEST("strncmp differ",   strncmp("abcX", "abcY", 4)  < 0);
    TEST("strncmp zero n",   strncmp("abc",  "xyz",  0) == 0);
}

// ---------------------------------------------------------------------------
// strcpy / strncpy
// ---------------------------------------------------------------------------

static void test_strcpy(void)
{
    printf("\nstrcpy / strncpy\n");
    char buf[16];
    strcpy(buf, "hello");
    TEST("copies string",     strcmp(buf, "hello") == 0);
    TEST("returns dst",       strcpy(buf, "x") == buf);

    char buf2[8] = {1,1,1,1,1,1,1,1};
    strncpy(buf2, "hi", 8);
    TEST("strncpy copies",    buf2[0]=='h' && buf2[1]=='i');
    TEST("strncpy pads null", buf2[2]=='\0' && buf2[7]=='\0');
}

// ---------------------------------------------------------------------------
// strchr / strrchr
// ---------------------------------------------------------------------------

static void test_strchr(void)
{
    printf("\nstrchr / strrchr\n");
    const char *s = "hello";
    TEST("strchr found first", strchr(s, 'l')  == s + 2);
    TEST("strchr null term",   strchr(s, '\0') == s + 5);
    TEST("strchr not found",   strchr(s, 'z')  == (char *)0);

    TEST("strrchr found last", strrchr(s, 'l') == s + 3);
    TEST("strrchr not found",  strrchr(s, 'z') == (char *)0);
}

// ---------------------------------------------------------------------------
// strcat / strncat
// ---------------------------------------------------------------------------

static void test_strcat(void)
{
    printf("\nstrcat / strncat\n");
    char buf[16] = "hello";
    strcat(buf, " world");
    TEST("strcat result",   strcmp(buf, "hello world") == 0);

    char buf2[16] = "foo";
    strncat(buf2, "barbaz", 3);
    TEST("strncat n chars", strcmp(buf2, "foobar") == 0);
    TEST("strncat null",    buf2[6] == '\0');
}

// ---------------------------------------------------------------------------
// strstr
// ---------------------------------------------------------------------------

static void test_strstr(void)
{
    printf("\nstrstr\n");
    const char *s = "hello world";
    TEST("found",         strstr(s, "world") == s + 6);
    TEST("not found",     strstr(s, "xyz")   == (char *)0);
    TEST("empty needle",  strstr(s, "")      == s);
    TEST("same string",   strstr("abc", "abc") != (char *)0);
}

// ---------------------------------------------------------------------------
// strspn / strcspn / strpbrk
// ---------------------------------------------------------------------------

static void test_span(void)
{
    printf("\nstrspn / strcspn / strpbrk\n");
    TEST("strspn full match",   strspn("abc", "abc")   == 3);
    TEST("strspn partial",      strspn("abcxyz", "abc") == 3);
    TEST("strspn no match",     strspn("xyz", "abc")   == 0);

    TEST("strcspn stops early", strcspn("abcxyz", "x") == 3);
    TEST("strcspn no reject",   strcspn("abc", "xyz")  == 3);

    const char *p = strpbrk("hello", "aeiou");
    TEST("strpbrk found",       p != (char *)0 && *p == 'e');
    TEST("strpbrk not found",   strpbrk("hello", "xyz") == (char *)0);
}

// ---------------------------------------------------------------------------
// strtok / strtok_r
// ---------------------------------------------------------------------------

static void test_strtok(void)
{
    printf("\nstrtok_r\n");
    char s[] = "one,two,,three";
    char *save;
    char *t = strtok_r(s, ",", &save);
    TEST("first token",   t != (char *)0 && strcmp(t, "one")   == 0);
    t = strtok_r(NULL, ",", &save);
    TEST("second token",  t != (char *)0 && strcmp(t, "two")   == 0);
    t = strtok_r(NULL, ",", &save);
    TEST("third token",   t != (char *)0 && strcmp(t, "three") == 0);
    t = strtok_r(NULL, ",", &save);
    TEST("exhausted",     t == (char *)0);
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

int main(void)
{
    printf("=== string tests ===\n");
    test_mem();
    test_strlen();
    test_strcmp();
    test_strcpy();
    test_strchr();
    test_strcat();
    test_strstr();
    test_span();
    test_strtok();
    printf("\n%d passed, %d failed\n", passed, failed);
    return failed > 0 ? 1 : 0;
}
