#pragma once

#include <stdint.h>

// Generational PID — a process identifier with a recycling counter.
//
// index:      the number visible to user space (1..pid_max()-1).
//             index 0 is permanently reserved (kernel use / "no PID").
// generation: incremented every time this index is recycled via pid_free.
//             Allows callers to detect stale PID handles: a stored
//             {index, generation} is stale if generation[index] differs.
//
// Both fields are uint16_t so the struct fits in a uint32_t register
// and the generation counter wraps naturally at 65535 (acceptable).
typedef struct {
    uint16_t index;       // visible PID number (0 = invalid / unallocated)
    uint16_t generation;  // recycling counter for this index
} pid_t;

// Initialize the PID allocator. Must be called after heap_init().
// Derives the PID limit from total installed RAM: roughly one PID slot
// for every process-sized chunk of memory. Clamped to [64, 65535].
void pid_init(uint64_t total_ram_bytes);

// Allocate a new PID. Returns {index, generation} for the slot.
// Returns {0, 0} if no free slot is available (PID table full).
pid_t pid_alloc(void);

// Release a PID back to the free pool.
// Increments the generation counter so any stored copies become stale.
// No-op if pid.index == 0 or pid.index >= pid_max().
void pid_free(pid_t pid);

// Return the current maximum number of PID slots (set at boot by pid_init).
uint32_t pid_max(void);
