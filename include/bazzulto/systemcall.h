#pragma once

#include <stdint.h>
#include "exceptions.h"

// System call numbers — passed in the SVC immediate (ISS[15:0] of ESR_EL1).
// User code invokes: svc #NR
#define SYS_EXIT    0
#define SYS_WRITE   1
#define SYS_READ    2
#define SYS_YIELD   3
#define NR_SYSTEMCALLS 4

// Dispatch a system call from the exception frame.
// Called from exception_handler_sync_el0 when EC = EC_SVC_AARCH64.
// Arguments are in frame->x0 through frame->x5.
// Return value is written to frame->x0 (restored by eret).
void systemcall_dispatch(struct exception_frame *frame);
