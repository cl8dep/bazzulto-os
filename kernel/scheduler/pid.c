#include "../../include/bazzulto/pid.h"
#include "../../include/bazzulto/heap.h"
#include "../../include/bazzulto/scheduler.h"  // sizeof(process_t) for limit calc

// ---------------------------------------------------------------------------
// PID bitmap allocator with generation counters
//
// Layout:
//   pid_bitmap      — one bit per slot; 0 = free, 1 = in use
//   pid_generation  — per-slot uint16_t, incremented on each pid_free
//   pid_limit       — total number of slots (set at boot, immutable after)
//   pid_next_hint   — start of next linear scan (avoids O(n) from slot 1 always)
//
// Index 0 is permanently reserved as "no PID / kernel" and is never returned
// by pid_alloc. This means user-space always sees PIDs >= 1.
// ---------------------------------------------------------------------------

static uint8_t  *pid_bitmap     = NULL;
static uint16_t *pid_generation = NULL;
static uint32_t  pid_limit      = 0;
static uint32_t  pid_next_hint  = 1;

// Per-process memory cost used to derive the PID limit:
//   kernel stack  = 16384 bytes (KERNEL_STACK_SIZE in scheduler.c)
//   process_t     = sizeof(process_t)
//   one page table page minimum = 4096 bytes
#define PID_PROCESS_COST (16384U + (uint32_t)sizeof(process_t) + 4096U)

#define PID_MIN 64U
#define PID_MAX 65535U   // index is uint16_t; 65535 is the max valid index

static void bitmap_set(uint32_t index) {
    pid_bitmap[index / 8] |= (uint8_t)(1U << (index % 8));
}

static void bitmap_clear(uint32_t index) {
    pid_bitmap[index / 8] &= (uint8_t)~(1U << (index % 8));
}

static int bitmap_test(uint32_t index) {
    return (pid_bitmap[index / 8] >> (index % 8)) & 1;
}

void pid_init(uint64_t total_ram_bytes) {
    // Compute limit: roughly one PID per process-sized chunk, using 1/4 of
    // total RAM as the "schedulable" portion (the rest is kernel/firmware/etc).
    uint64_t schedulable = total_ram_bytes / 4;
    uint64_t computed    = schedulable / PID_PROCESS_COST;

    uint32_t limit = (uint32_t)(computed < PID_MIN ? PID_MIN :
                                computed > PID_MAX ? PID_MAX : computed);

    pid_limit = limit;

    // Bitmap: one bit per slot, rounded up to whole bytes.
    uint32_t bitmap_bytes = (limit + 7) / 8;
    pid_bitmap = (uint8_t *)kmalloc(bitmap_bytes);
    if (!pid_bitmap) {
        // Allocation failure during boot — fall back to minimum size using a
        // static emergency buffer. This should never happen in practice.
        static uint8_t emergency_bitmap[PID_MIN / 8];
        pid_bitmap = emergency_bitmap;
        pid_limit  = PID_MIN;
        bitmap_bytes = PID_MIN / 8;
    }

    // Zero the bitmap (all slots free).
    for (uint32_t i = 0; i < (pid_limit + 7) / 8; i++)
        pid_bitmap[i] = 0;

    // Generation counters: one uint16_t per slot.
    pid_generation = (uint16_t *)kmalloc(pid_limit * sizeof(uint16_t));
    if (!pid_generation) {
        static uint16_t emergency_generation[PID_MIN];
        pid_generation = emergency_generation;
    }
    for (uint32_t i = 0; i < pid_limit; i++)
        pid_generation[i] = 0;

    // Reserve index 0 permanently (kernel sentinel / "no PID").
    bitmap_set(0);

    pid_next_hint = 1;
}

pid_t pid_alloc(void) {
    if (!pid_bitmap) {
        pid_t none = {0, 0};
        return none;
    }

    // Linear scan from next_hint, wrapping around to 1 (skip index 0).
    uint32_t start = pid_next_hint;
    uint32_t index = start;

    do {
        if (!bitmap_test(index)) {
            bitmap_set(index);
            pid_next_hint = (index + 1 < pid_limit) ? index + 1 : 1;
            pid_t result = {(uint16_t)index, pid_generation[index]};
            return result;
        }
        index++;
        if (index >= pid_limit)
            index = 1;  // wrap, never visit 0
    } while (index != start);

    // No free slot found.
    pid_t none = {0, 0};
    return none;
}

void pid_free(pid_t pid) {
    if (pid.index == 0 || pid.index >= pid_limit)
        return;

    bitmap_clear(pid.index);
    // Increment generation so any stored copies of this PID become stale.
    // uint16_t wraps naturally at 65535 — that is intentional.
    pid_generation[pid.index]++;
}

uint32_t pid_max(void) {
    return pid_limit;
}
