/* test_signal.c — sigaction, raise, sigprocmask */
#include <stdio.h>
#include <signal.h>
#include <string.h>

static volatile int handler_called = 0;
static volatile int handler_signum = 0;

static void handler(int sig) {
    handler_called = 1;
    handler_signum = sig;
}

int main(void) {
    int pass = 1;

    /* Install handler for SIGUSR1 */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = handler;
    if (sigaction(SIGUSR1, &sa, NULL) != 0) { puts("FAIL sigaction"); pass = 0; }

    /* Raise SIGUSR1 */
    raise(SIGUSR1);
    if (!handler_called) { puts("FAIL raise — handler not called"); pass = 0; }
    if (handler_signum != SIGUSR1) { puts("FAIL raise — wrong signal"); pass = 0; }

    /* sigprocmask — block SIGUSR1, raise, verify not delivered */
    handler_called = 0;
    sigset_t mask, oldmask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);
    sigprocmask(SIG_BLOCK, &mask, &oldmask);

    raise(SIGUSR1);
    if (handler_called) { puts("FAIL sigprocmask — signal delivered while blocked"); pass = 0; }

    /* Unblock — signal should be delivered now */
    sigprocmask(SIG_SETMASK, &oldmask, NULL);
    if (!handler_called) { puts("FAIL sigprocmask — signal not delivered after unblock"); pass = 0; }

    if (pass) puts("PASS test_signal");
    return pass ? 0 : 1;
}
