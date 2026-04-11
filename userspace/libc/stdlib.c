#include "stdlib.h"
#include "string.h"

// exit() and mmap()/munmap() are provided by userspace/library/systemcall.S;
// forward-declare here to avoid pulling in the full syscall header.
extern void  exit(int status) __attribute__((noreturn));
extern void *mmap(size_t length);
extern int   munmap(void *addr);

// ---------------------------------------------------------------------------
// Core conversion — strtoull
// Handles: leading whitespace, optional sign, base-0 auto-detect
// (0x/0X → hex, leading 0 → octal, else decimal), overflow clamping.
// No errno — document in header.
// ---------------------------------------------------------------------------

unsigned long long strtoull(const char *str, char **endptr, int base)
{
    const char *cursor = str;

    // Skip leading whitespace.
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\n' ||
           *cursor == '\r' || *cursor == '\f' || *cursor == '\v')
        cursor++;

    // Optional sign (unsigned result — '-' wraps, '+' ignored).
    int negative = 0;
    if (*cursor == '-') { negative = 1; cursor++; }
    else if (*cursor == '+') { cursor++; }

    // Base auto-detection.
    if (base == 0) {
        if (cursor[0] == '0' && (cursor[1] == 'x' || cursor[1] == 'X')) {
            base = 16;
        } else if (cursor[0] == '0') {
            base = 8;
        } else {
            base = 10;
        }
    }

    // Skip '0x' / '0X' prefix for base 16.
    if (base == 16 && cursor[0] == '0' &&
        (cursor[1] == 'x' || cursor[1] == 'X'))
        cursor += 2;

    // Accumulate digits.
    unsigned long long result   = 0;
    unsigned long long overflow = (unsigned long long)-1;
    int any_digits = 0;

    while (1) {
        int digit;
        char character = *cursor;
        if (character >= '0' && character <= '9')
            digit = character - '0';
        else if (character >= 'a' && character <= 'z')
            digit = character - 'a' + 10;
        else if (character >= 'A' && character <= 'Z')
            digit = character - 'A' + 10;
        else
            break;

        if (digit >= base)
            break;

        // Overflow check: if result > (ULLONG_MAX - digit) / base, clamp.
        if (result > (overflow - (unsigned long long)digit) / (unsigned long long)base) {
            result = overflow;
        } else {
            result = result * (unsigned long long)base + (unsigned long long)digit;
        }
        any_digits = 1;
        cursor++;
    }

    if (endptr)
        *endptr = (char *)(any_digits ? cursor : str);

    return negative ? (~result + 1ULL) : result;  // two's-complement negation
}

// ---------------------------------------------------------------------------
// Signed variants — delegate to strtoull with clamping.
// ---------------------------------------------------------------------------

long long strtoll(const char *str, char **endptr, int base)
{
    const char *cursor = str;

    // Skip whitespace and detect sign before delegating to strtoull.
    // We must NOT pass the '-' to strtoull because strtoull applies
    // two's-complement negation itself, which would confuse our clamping.
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '\n' ||
           *cursor == '\r' || *cursor == '\f' || *cursor == '\v')
        cursor++;

    int negative = 0;
    if (*cursor == '-') { negative = 1; cursor++; }
    else if (*cursor == '+') { cursor++; }

    // Parse the magnitude (unsigned, no sign).
    unsigned long long magnitude = strtoull(cursor, endptr, base);

    // LLONG_MAX = 0x7FFFFFFFFFFFFFFF, LLONG_MIN magnitude = 0x8000000000000000
    if (negative) {
        if (magnitude > 0x8000000000000000ULL)
            return (long long)0x8000000000000000ULL;  // clamp to LLONG_MIN
        return -(long long)magnitude;
    } else {
        if (magnitude > 0x7FFFFFFFFFFFFFFFULL)
            return (long long)0x7FFFFFFFFFFFFFFFULL;  // clamp to LLONG_MAX
        return (long long)magnitude;
    }
}

long strtol(const char *str, char **endptr, int base)
{
    long long result = strtoll(str, endptr, base);
    // LONG_MAX = 0x7FFFFFFF on 32-bit; on 64-bit AArch64, long is 64-bit.
    return (long)result;
}

unsigned long strtoul(const char *str, char **endptr, int base)
{
    return (unsigned long)strtoull(str, endptr, base);
}

// ---------------------------------------------------------------------------
// Convenience wrappers
// ---------------------------------------------------------------------------

int atoi(const char *str)
{
    return (int)strtol(str, (char **)0, 10);
}

long atol(const char *str)
{
    return strtol(str, (char **)0, 10);
}

long long atoll(const char *str)
{
    return strtoll(str, (char **)0, 10);
}

// ---------------------------------------------------------------------------
// Integer arithmetic
// ---------------------------------------------------------------------------

int abs(int n)
{
    return n < 0 ? -n : n;
}

long labs(long n)
{
    return n < 0 ? -n : n;
}

long long llabs(long long n)
{
    return n < 0 ? -n : n;
}

// ---------------------------------------------------------------------------
// abort — terminate immediately (no atexit, no flushing)
// ---------------------------------------------------------------------------

void abort(void)
{
    exit(134);  // 128 + SIGABRT(6) = conventional abort exit code
}

// ---------------------------------------------------------------------------
// Pseudo-random number generation — LCG (Numerical Recipes constants)
// ---------------------------------------------------------------------------

static unsigned int rand_state = 1;

int rand(void)
{
    rand_state = rand_state * 1664525u + 1013904223u;
    return (int)((rand_state >> 1) & 0x7fffffff);
}

void srand(unsigned int seed)
{
    rand_state = seed;
}

// ---------------------------------------------------------------------------
// bsearch — binary search (C11 §7.22.5.1)
// ---------------------------------------------------------------------------

void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compare)(const void *, const void *))
{
    size_t lo = 0, hi = nmemb;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const void *elem = (const char *)base + mid * size;
        int cmp = compare(key, elem);
        if (cmp == 0) return (void *)elem;
        if (cmp < 0)  hi = mid;
        else          lo = mid + 1;
    }
    return (void *)0;
}

// ---------------------------------------------------------------------------
// qsort — insertion sort for small arrays, recursive quicksort otherwise
// ---------------------------------------------------------------------------

static void swap(char *a, char *b, size_t size)
{
    while (size--) {
        char tmp = *a;
        *a++ = *b;
        *b++ = tmp;
    }
}

static void insertion_sort(char *base, size_t nmemb, size_t size,
                            int (*cmp)(const void *, const void *))
{
    for (size_t i = 1; i < nmemb; i++) {
        size_t j = i;
        while (j > 0 && cmp(base + (j-1)*size, base + j*size) > 0) {
            swap(base + (j-1)*size, base + j*size, size);
            j--;
        }
    }
}

// ---------------------------------------------------------------------------
// malloc — first-fit allocator backed by mmap
//
// Block header layout (32 bytes, keeps returned pointers 32-byte aligned):
//
//   offset  0: size_t usable_size  — bytes available after this header
//   offset  8: size_t flags        — bit 0: 1=free, 0=allocated
//   offset 16: struct malloc_block *next_free  — next in free list (NULL if end)
//   offset 24: (reserved / padding to 32 bytes)
//
// The free list is singly linked (next_free). Allocation uses first-fit.
// On malloc, if the found free block is large enough, it is split if the
// leftover is >= MALLOC_HEADER_SIZE + MALLOC_MIN_USABLE bytes.
// On free, the block is prepended to the free list (no coalescing — simple
// but correct; fragmentation grows over time but is acceptable for a hobby OS).
// New memory is requested from the kernel via mmap() in MALLOC_SLAB_PAGES
// increments (currently 16 pages = 64 KB per slab request).
// ---------------------------------------------------------------------------

typedef struct malloc_block {
    size_t                usable_size;  // caller-usable bytes following this header
    size_t                flags;        // bit 0: MALLOC_FLAG_FREE
    struct malloc_block  *next_free;    // next free block, NULL if tail
    size_t                _reserved;    // padding to 32 bytes
} malloc_block_t;

#define MALLOC_FLAG_FREE      1u
#define MALLOC_HEADER_SIZE    sizeof(malloc_block_t)  // 32 bytes
#define MALLOC_MIN_USABLE     32u    // minimum usable bytes in a split-off block
#define MALLOC_SLAB_PAGES     16u    // pages per mmap slab (64 KB)
#define PAGE_SIZE_BYTES       4096u

static malloc_block_t *malloc_free_head = (malloc_block_t *)0;

static void *malloc_new_slab(size_t minimum_total)
{
    size_t slab_bytes = MALLOC_SLAB_PAGES * PAGE_SIZE_BYTES;
    if (minimum_total > slab_bytes)
        slab_bytes = ((minimum_total + PAGE_SIZE_BYTES - 1) /
                      PAGE_SIZE_BYTES) * PAGE_SIZE_BYTES;
    return mmap(slab_bytes);
}

void *malloc(size_t size)
{
    if (size == 0)
        return (void *)0;

    // Round up to 32-byte alignment so every allocation is 32-byte aligned.
    size = (size + 31u) & ~(size_t)31u;

    // Search the free list for a first-fit block.
    malloc_block_t *prev = (malloc_block_t *)0;
    malloc_block_t *block = malloc_free_head;
    while (block) {
        if ((block->flags & MALLOC_FLAG_FREE) && block->usable_size >= size) {
            // Found a fit.  Unlink from free list.
            if (prev)
                prev->next_free = block->next_free;
            else
                malloc_free_head = block->next_free;
            block->next_free = (malloc_block_t *)0;

            // Split if the remainder is large enough to be useful.
            size_t remainder = block->usable_size - size;
            if (remainder >= MALLOC_HEADER_SIZE + MALLOC_MIN_USABLE) {
                malloc_block_t *split = (malloc_block_t *)
                    ((char *)block + MALLOC_HEADER_SIZE + size);
                split->usable_size = remainder - MALLOC_HEADER_SIZE;
                split->flags       = MALLOC_FLAG_FREE;
                split->next_free   = malloc_free_head;
                split->_reserved   = 0;
                malloc_free_head   = split;
                block->usable_size = size;
            }

            block->flags = 0;  // mark allocated
            return (char *)block + MALLOC_HEADER_SIZE;
        }
        prev  = block;
        block = block->next_free;
    }

    // No suitable block — request a new slab from the kernel.
    size_t slab_bytes = MALLOC_SLAB_PAGES * PAGE_SIZE_BYTES;
    size_t needed     = MALLOC_HEADER_SIZE + size;
    if (needed > slab_bytes)
        slab_bytes = ((needed + PAGE_SIZE_BYTES - 1) / PAGE_SIZE_BYTES) *
                     PAGE_SIZE_BYTES;

    void *raw = malloc_new_slab(slab_bytes);
    if (!raw || raw == (void *)-1)
        return (void *)0;

    // Carve a free block out of the slab and allocate from it.
    malloc_block_t *new_block = (malloc_block_t *)raw;
    new_block->usable_size = slab_bytes - MALLOC_HEADER_SIZE;
    new_block->flags       = MALLOC_FLAG_FREE;
    new_block->next_free   = malloc_free_head;
    new_block->_reserved   = 0;
    malloc_free_head       = new_block;

    // Recurse once — the new slab is now in the free list.
    return malloc(size);
}

void free(void *ptr)
{
    if (!ptr)
        return;

    malloc_block_t *block = (malloc_block_t *)((char *)ptr - MALLOC_HEADER_SIZE);

    // Guard against double-free (best-effort — no canonical free list check).
    if (block->flags & MALLOC_FLAG_FREE)
        return;

    block->flags     = MALLOC_FLAG_FREE;
    block->next_free = malloc_free_head;
    malloc_free_head = block;
}

void *calloc(size_t nmemb, size_t size)
{
    // Check for multiplication overflow.
    if (nmemb && size > (size_t)-1 / nmemb)
        return (void *)0;
    size_t total = nmemb * size;
    void *ptr = malloc(total);
    if (ptr)
        memset(ptr, 0, total);
    return ptr;
}

void *realloc(void *ptr, size_t new_size)
{
    if (!ptr)
        return malloc(new_size);
    if (new_size == 0) {
        free(ptr);
        return (void *)0;
    }

    malloc_block_t *block = (malloc_block_t *)((char *)ptr - MALLOC_HEADER_SIZE);
    size_t rounded = (new_size + 31u) & ~(size_t)31u;

    // Current block is large enough — reuse it.
    if (block->usable_size >= rounded)
        return ptr;

    // Need a bigger block — allocate, copy, free old.
    void *new_ptr = malloc(new_size);
    if (!new_ptr)
        return (void *)0;
    memcpy(new_ptr, ptr, block->usable_size < new_size ? block->usable_size : new_size);
    free(ptr);
    return new_ptr;
}

void qsort(void *base, size_t nmemb, size_t size,
           int (*compare)(const void *, const void *))
{
    if (nmemb < 16) {
        insertion_sort((char *)base, nmemb, size, compare);
        return;
    }

    // Median-of-three pivot.
    size_t mid = nmemb / 2;
    char *b = (char *)base;
    if (compare(b, b + mid*size) > 0)            swap(b, b + mid*size, size);
    if (compare(b, b + (nmemb-1)*size) > 0)      swap(b, b + (nmemb-1)*size, size);
    if (compare(b + mid*size, b + (nmemb-1)*size) > 0)
        swap(b + mid*size, b + (nmemb-1)*size, size);

    // Place pivot at nmemb-2.
    swap(b + mid*size, b + (nmemb-2)*size, size);
    char *pivot = b + (nmemb-2)*size;

    size_t lo = 1, hi = nmemb - 2;
    while (1) {
        while (compare(b + lo*size, pivot) < 0) lo++;
        while (compare(b + hi*size, pivot) > 0) hi--;
        if (lo >= hi) break;
        swap(b + lo*size, b + hi*size, size);
        lo++; hi--;
    }
    swap(b + lo*size, pivot, size);

    qsort(b, lo, size, compare);
    qsort(b + (lo+1)*size, nmemb - lo - 1, size, compare);
}
