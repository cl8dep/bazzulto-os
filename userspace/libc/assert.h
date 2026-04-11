#pragma once

// assert — C11 §7.2
// If NDEBUG is defined before including this header, assert() is a no-op.
// Otherwise, a failed assertion calls abort() after printing diagnostics.
//
// Note: in a freestanding kernel environment, the diagnostic message is
// limited to what printf() can output. There is no stderr — output goes
// to stdout (fd 1).

#ifdef NDEBUG

#define assert(expr) ((void)0)

#else

// abort() is declared in stdlib.h; forward-declare here to avoid
// requiring the user to include stdlib.h just for assert.
extern void abort(void) __attribute__((noreturn));

// printf() is declared in stdio.h; forward-declare here for the
// same reason.
extern int printf(const char *format, ...)
    __attribute__((format(printf, 1, 2)));

#define assert(expr)                                                      \
    do {                                                                  \
        if (!(expr)) {                                                    \
            printf("assert failed: %s, file %s, line %d\n",              \
                   #expr, __FILE__, __LINE__);                            \
            abort();                                                      \
        }                                                                 \
    } while (0)

#endif
