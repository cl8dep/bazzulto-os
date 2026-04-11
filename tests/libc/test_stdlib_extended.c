// Tests for stdlib.c extended functions: rand, srand, bsearch, qsort, malloc, free, calloc, realloc
// Compile and run on the host: cc -o /tmp/test_stdlib_ext tests/libc/test_stdlib_extended.c userspace/libc/stdlib.c userspace/libc/string.c -I userspace/libc && /tmp/test_stdlib_ext

#include "../../userspace/libc/stdlib.h"
#include "../../userspace/libc/string.h"
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

// ---------------------------------------------------------------------------
// rand / srand
// ---------------------------------------------------------------------------

static void test_rand(void)
{
    printf("\nrand / srand\n");

    // Same seed → same sequence
    srand(42);
    int a1 = rand();
    int a2 = rand();
    srand(42);
    int b1 = rand();
    int b2 = rand();
    TEST("same seed same sequence",   a1 == b1 && a2 == b2);
    TEST("consecutive values differ", a1 != a2);
    TEST("result >= 0",               a1 >= 0);
    TEST("result <= RAND_MAX",        a1 <= RAND_MAX);

    // Different seed → different sequence
    srand(1);
    int c1 = rand();
    srand(999);
    int d1 = rand();
    TEST("different seed different value", c1 != d1);
}

// ---------------------------------------------------------------------------
// bsearch
// ---------------------------------------------------------------------------

static int compare_int(const void *a, const void *b)
{
    return *(const int *)a - *(const int *)b;
}

static void test_bsearch(void)
{
    printf("\nbsearch\n");

    int arr[] = { 2, 5, 8, 12, 16, 23, 38, 56, 72, 91 };
    int n = (int)(sizeof(arr) / sizeof(arr[0]));

    int key;

    key = 23;
    int *found = (int *)bsearch(&key, arr, (size_t)n, sizeof(int), compare_int);
    TEST("find 23",            found != NULL && *found == 23);

    key = 2;
    found = (int *)bsearch(&key, arr, (size_t)n, sizeof(int), compare_int);
    TEST("find first element", found != NULL && *found == 2);

    key = 91;
    found = (int *)bsearch(&key, arr, (size_t)n, sizeof(int), compare_int);
    TEST("find last element",  found != NULL && *found == 91);

    key = 42;
    found = (int *)bsearch(&key, arr, (size_t)n, sizeof(int), compare_int);
    TEST("not found returns NULL", found == NULL);

    key = 5;
    found = (int *)bsearch(&key, arr, 0, sizeof(int), compare_int);
    TEST("empty array returns NULL", found == NULL);
}

// ---------------------------------------------------------------------------
// qsort
// ---------------------------------------------------------------------------

static void test_qsort(void)
{
    printf("\nqsort\n");

    // Small array (insertion sort path)
    {
        int arr[] = { 5, 3, 1, 4, 2 };
        qsort(arr, 5, sizeof(int), compare_int);
        TEST("small sort [0]=1", arr[0] == 1);
        TEST("small sort [4]=5", arr[4] == 5);
        int sorted = 1;
        for (int i = 1; i < 5; i++) if (arr[i] < arr[i-1]) sorted = 0;
        TEST("small sort fully sorted", sorted);
    }

    // Larger array (quicksort path, > 16 elements)
    {
        int arr[] = { 99, 3, 55, 7, 22, 88, 1, 44, 66, 11,
                      33, 77, 5, 9, 100, 2, 50, 15, 80, 42 };
        int n = 20;
        qsort(arr, (size_t)n, sizeof(int), compare_int);
        TEST("large sort [0]=1",    arr[0] == 1);
        TEST("large sort [19]=100", arr[19] == 100);
        int sorted = 1;
        for (int i = 1; i < n; i++) if (arr[i] < arr[i-1]) sorted = 0;
        TEST("large sort fully sorted", sorted);
    }

    // Already sorted
    {
        int arr[] = { 1, 2, 3, 4, 5 };
        qsort(arr, 5, sizeof(int), compare_int);
        TEST("already sorted stays sorted", arr[0] == 1 && arr[4] == 5);
    }

    // Reverse sorted
    {
        int arr[] = { 5, 4, 3, 2, 1 };
        qsort(arr, 5, sizeof(int), compare_int);
        TEST("reverse sorted becomes sorted", arr[0] == 1 && arr[4] == 5);
    }

    // All same
    {
        int arr[] = { 7, 7, 7, 7, 7 };
        qsort(arr, 5, sizeof(int), compare_int);
        TEST("all same stays same", arr[0] == 7 && arr[4] == 7);
    }

    // Single element
    {
        int arr[] = { 42 };
        qsort(arr, 1, sizeof(int), compare_int);
        TEST("single element", arr[0] == 42);
    }

    // Empty
    {
        int arr[] = { 1 };
        qsort(arr, 0, sizeof(int), compare_int);
        TEST("empty does nothing", arr[0] == 1);
    }
}

// ---------------------------------------------------------------------------
// malloc / free / calloc / realloc
// Note: these tests only run if malloc is backed by the host system's mmap.
// On the host, our stdlib.c's malloc calls mmap() which links to the host OS.
// ---------------------------------------------------------------------------

// malloc / free / calloc / realloc tests are skipped on the host.
// Our malloc calls mmap() which is a Bazzulto kernel syscall — it does not
// link against the host OS mmap. These tests can only run inside the OS.
static void test_malloc(void)
{
    printf("\nmalloc / free / calloc / realloc\n");
    printf("  SKIP  (requires Bazzulto mmap syscall — cannot run on host)\n");

    // Only test behaviors that don't require mmap:
    void *z = malloc(0);
    TEST("malloc(0) returns NULL", z == NULL);

    void *overflow = calloc((size_t)-1, (size_t)-1);
    TEST("calloc overflow returns NULL", overflow == NULL);

    free(NULL);
    TEST("free(NULL) no crash", 1);
}

int main(void)
{
    printf("=== stdlib extended tests ===\n");
    test_rand();
    test_bsearch();
    test_qsort();
    test_malloc();
    printf("\n%d passed, %d failed\n", passed, failed);
    return failed ? 1 : 0;
}
