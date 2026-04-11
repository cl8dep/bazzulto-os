# Technical Debt — Scheduler

## Zombie Count Limit — DONE
`process_t.zombie_count` (uint16_t) tracks un-reaped zombie children.
`sys_spawn` refuses with -1 when count reaches `ZOMBIE_COUNT_MAX = 64`.
`sys_exit` increments parent's count; `sys_wait` decrements it.

## wait(-1) / Any Child — DONE
`sys_wait` accepts -1 as a sentinel meaning "wait for any child".
`scheduler_find_zombie_child` and `scheduler_has_child` support this.
`waiting_for_pid = 0xFFFF` is the in-process sentinel for "waiting for any child".

## fork() — DONE
`scheduler_fork_process(parent_frame)` deep-copies the user address space
(all L3 leaf pages) via `virtual_memory_deep_copy_table`, copies the exception
frame onto the child's kernel stack with x0=0, and sets context.x30 to
`fork_child_resume` (asm: expands `restore_exception_frame_el0` → eret to EL0).

## exec() — DONE
`scheduler_free_user_address_space` frees the TTBR0 without touching the kernel
stack or process struct. `sys_exec` calls `elf_loader_build_image`, installs the
new page table, and rewrites the exception frame (ELR, SP_EL0, SPSR) so that
the eret returns to the new entry point.

## Round Robin → MLFQ — OPEN
Scheduler is still round-robin. MLFQ (multi-level feedback queue) is needed
for interactive responsiveness. `process_t` would gain `priority` and
`ticks_used` fields; the run queue becomes an array of priority queues.

## SMP — OPEN
Single-core only. Multi-core support requires per-core run queues, a load
balancer, and spinlocks on all shared scheduler state. Implement after the
single-core scheduler is stable and correct under load.
