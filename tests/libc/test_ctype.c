// Tests for userspace/libc/ctype.c
// Compile and run on the host: cc -o /tmp/test_ctype tests/libc/test_ctype.c userspace/libc/ctype.c && /tmp/test_ctype

#include "../../userspace/libc/ctype.h"
#include <stdio.h>

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

static void test_isalpha(void)
{
    printf("\nisalpha\n");
    TEST("'A' is alpha",       isalpha('A'));
    TEST("'z' is alpha",       isalpha('z'));
    TEST("'0' is not alpha",  !isalpha('0'));
    TEST("' ' is not alpha",  !isalpha(' '));
    TEST("'@' is not alpha",  !isalpha('@'));
}

static void test_isdigit(void)
{
    printf("\nisdigit\n");
    TEST("'0' is digit",       isdigit('0'));
    TEST("'9' is digit",       isdigit('9'));
    TEST("'a' is not digit",  !isdigit('a'));
    TEST("'/' is not digit",  !isdigit('/'));
    TEST("':' is not digit",  !isdigit(':'));
}

static void test_isalnum(void)
{
    printf("\nisalnum\n");
    TEST("'A' is alnum",       isalnum('A'));
    TEST("'5' is alnum",       isalnum('5'));
    TEST("'!' is not alnum",  !isalnum('!'));
}

static void test_isspace(void)
{
    printf("\nisspace\n");
    TEST("' ' is space",       isspace(' '));
    TEST("'\\t' is space",     isspace('\t'));
    TEST("'\\n' is space",     isspace('\n'));
    TEST("'\\r' is space",     isspace('\r'));
    TEST("'\\f' is space",     isspace('\f'));
    TEST("'\\v' is space",     isspace('\v'));
    TEST("'a' is not space",  !isspace('a'));
}

static void test_isupper_islower(void)
{
    printf("\nisupper / islower\n");
    TEST("'A' is upper",       isupper('A'));
    TEST("'Z' is upper",       isupper('Z'));
    TEST("'a' is not upper",  !isupper('a'));
    TEST("'a' is lower",       islower('a'));
    TEST("'z' is lower",       islower('z'));
    TEST("'A' is not lower",  !islower('A'));
}

static void test_isprint_isgraph(void)
{
    printf("\nisprint / isgraph\n");
    TEST("' ' is print",       isprint(' '));
    TEST("'~' is print",       isprint('~'));
    TEST("0x1F is not print", !isprint(0x1F));
    TEST("0x7F is not print", !isprint(0x7F));
    TEST("'!' is graph",       isgraph('!'));
    TEST("' ' is not graph",  !isgraph(' '));
}

static void test_ispunct(void)
{
    printf("\nispunct\n");
    TEST("'!' is punct",       ispunct('!'));
    TEST("'.' is punct",       ispunct('.'));
    TEST("'a' is not punct",  !ispunct('a'));
    TEST("'5' is not punct",  !ispunct('5'));
    TEST("' ' is not punct",  !ispunct(' '));
}

static void test_iscntrl(void)
{
    printf("\niscntrl\n");
    TEST("0x00 is cntrl",      iscntrl(0x00));
    TEST("0x1F is cntrl",      iscntrl(0x1F));
    TEST("0x7F is cntrl",      iscntrl(0x7F));
    TEST("' ' is not cntrl",  !iscntrl(' '));
}

static void test_isxdigit(void)
{
    printf("\nisxdigit\n");
    TEST("'0' is xdigit",      isxdigit('0'));
    TEST("'9' is xdigit",      isxdigit('9'));
    TEST("'a' is xdigit",      isxdigit('a'));
    TEST("'f' is xdigit",      isxdigit('f'));
    TEST("'A' is xdigit",      isxdigit('A'));
    TEST("'F' is xdigit",      isxdigit('F'));
    TEST("'g' is not xdigit", !isxdigit('g'));
    TEST("'G' is not xdigit", !isxdigit('G'));
}

static void test_isblank(void)
{
    printf("\nisblank\n");
    TEST("' ' is blank",       isblank(' '));
    TEST("'\\t' is blank",     isblank('\t'));
    TEST("'\\n' is not blank", !isblank('\n'));
}

static void test_toupper_tolower(void)
{
    printf("\ntoupper / tolower\n");
    TEST("toupper('a') = 'A'",  toupper('a') == 'A');
    TEST("toupper('z') = 'Z'",  toupper('z') == 'Z');
    TEST("toupper('A') = 'A'",  toupper('A') == 'A');
    TEST("toupper('5') = '5'",  toupper('5') == '5');
    TEST("tolower('A') = 'a'",  tolower('A') == 'a');
    TEST("tolower('Z') = 'z'",  tolower('Z') == 'z');
    TEST("tolower('a') = 'a'",  tolower('a') == 'a');
    TEST("tolower('5') = '5'",  tolower('5') == '5');
}

int main(void)
{
    printf("=== ctype tests ===\n");
    test_isalpha();
    test_isdigit();
    test_isalnum();
    test_isspace();
    test_isupper_islower();
    test_isprint_isgraph();
    test_ispunct();
    test_iscntrl();
    test_isxdigit();
    test_isblank();
    test_toupper_tolower();
    printf("\n%d passed, %d failed\n", passed, failed);
    return failed ? 1 : 0;
}
