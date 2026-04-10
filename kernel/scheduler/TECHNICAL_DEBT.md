# Technical Debt — Scheduler

## Round Robin → Multi-Level Feedback Queue (MLFQ)

**Current implementation:** simple Round Robin — all processes share equal CPU time
in fixed-size time slices, cycling through a circular list.

**Why migrate:** Round Robin treats all processes equally, but in practice processes
have different needs:
- Interactive processes (UI, shell) need low latency — they should preempt quickly
- Background processes (compilation, compression) are CPU-bound — latency matters less
- Real-time processes (audio, sensors) need guaranteed response times

**Target: MLFQ** (as used by macOS/XNU and early Windows NT)

How it works:
- Multiple queues, each with a different priority level (e.g. 0=realtime, 1=high,
  2=normal, 3=background)
- New processes start at high priority
- If a process uses its full time slice without blocking, it moves DOWN one level
  (signal: it's CPU-bound, less urgent)
- If a process blocks early (waiting for I/O, input), it stays at high priority
  (signal: it's interactive, needs responsiveness)
- The scheduler always picks from the highest non-empty queue

**What needs to change:**
- `struct process` gains a `priority` field and a `ticks_used` counter
- The ready list becomes an array of lists, one per priority level
- `scheduler_tick` checks ticks_used and demotes the process if it exhausted its slice
- `scheduler_unblock` (called when I/O completes) bumps priority back up

**Migration effort:** medium. The context switch mechanism stays identical.
Only the data structure and selection logic change.

## Single-core only

The current scheduler assumes one CPU. Supporting multiple cores (SMP) requires:
- Per-core run queues to avoid lock contention
- A load balancer that migrates processes between cores
- Spinlocks on all shared scheduler state

This is a significant redesign. Implement after the single-core scheduler is stable.
