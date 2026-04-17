/* test_time.c — clock_gettime, nanosleep */
#include <stdio.h>
#include <time.h>

int main(void) {
    int pass = 1;

    /* clock_gettime CLOCK_REALTIME — should return epoch > 2024 */
    struct timespec rt;
    if (clock_gettime(CLOCK_REALTIME, &rt) != 0) {
        puts("FAIL clock_gettime REALTIME");
        pass = 0;
    } else if (rt.tv_sec < 1700000000) {
        printf("FAIL REALTIME too low: %ld\n", (long)rt.tv_sec);
        pass = 0;
    }

    /* clock_gettime CLOCK_MONOTONIC — two reads, second >= first */
    struct timespec m1, m2;
    clock_gettime(CLOCK_MONOTONIC, &m1);
    clock_gettime(CLOCK_MONOTONIC, &m2);
    if (m2.tv_sec < m1.tv_sec ||
        (m2.tv_sec == m1.tv_sec && m2.tv_nsec < m1.tv_nsec)) {
        puts("FAIL MONOTONIC went backwards");
        pass = 0;
    }

    /* nanosleep 100ms — verify at least 50ms elapsed */
    struct timespec before, after;
    clock_gettime(CLOCK_MONOTONIC, &before);

    struct timespec req = { .tv_sec = 0, .tv_nsec = 100000000 }; /* 100ms */
    nanosleep(&req, NULL);

    clock_gettime(CLOCK_MONOTONIC, &after);
    long elapsed_ms = (after.tv_sec - before.tv_sec) * 1000
                    + (after.tv_nsec - before.tv_nsec) / 1000000;
    if (elapsed_ms < 50) {
        printf("FAIL nanosleep too short: %ld ms\n", elapsed_ms);
        pass = 0;
    }

    if (pass) puts("PASS test_time");
    return pass ? 0 : 1;
}
