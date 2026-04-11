#include "errno.h"

// Per-process errno — single-threaded for now.
// When threads are added, change to __thread int errno.
int errno = 0;
