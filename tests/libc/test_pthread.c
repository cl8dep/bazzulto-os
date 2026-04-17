/* test_pthread.c — threads with mutex */
#include <stdio.h>
#include <pthread.h>

#define NUM_THREADS    4
#define INCREMENTS     10000

static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static int counter = 0;

static void *worker(void *arg) {
    (void)arg;
    for (int i = 0; i < INCREMENTS; i++) {
        pthread_mutex_lock(&mutex);
        counter++;
        pthread_mutex_unlock(&mutex);
    }
    return NULL;
}

int main(void) {
    int pass = 1;
    pthread_t threads[NUM_THREADS];

    for (int i = 0; i < NUM_THREADS; i++) {
        if (pthread_create(&threads[i], NULL, worker, NULL) != 0) {
            printf("FAIL pthread_create %d\n", i);
            pass = 0;
        }
    }

    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(threads[i], NULL);
    }

    int expected = NUM_THREADS * INCREMENTS;
    if (counter != expected) {
        printf("FAIL counter = %d, expected %d\n", counter, expected);
        pass = 0;
    }

    if (pass) puts("PASS test_pthread");
    return pass ? 0 : 1;
}
