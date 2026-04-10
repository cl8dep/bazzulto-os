#pragma once

#include "scheduler.h"

// A wait queue is a list of processes blocked waiting for an event
// (e.g. data available on UART RX). When the event occurs, one or
// all waiting processes are woken up and moved back to READY state.
//
// Usage pattern (reader side):
//     __asm__ volatile("msr daifset, #2");   // disable IRQs
//     while (!data_available)
//         process_sleep(&my_wq);             // blocks, re-enables IRQs
//     // ... consume data ...
//     __asm__ volatile("msr daifclr, #2");   // re-enable IRQs
//
// Usage pattern (IRQ handler / producer side):
//     // ... produce data ...
//     process_wakeup(&my_wq);                // wakes one waiter

typedef struct wait_queue {
	process_t *head;  // singly-linked list of blocked processes
} wait_queue_t;

#define WAIT_QUEUE_INIT { .head = NULL }

// Block the current process on this wait queue.
// The caller MUST disable IRQs before calling (msr daifset, #2).
// Returns after another context wakes this process up.
void process_sleep(wait_queue_t *wq);

// Wake one process from the wait queue (FIFO order).
// Safe to call from IRQ context.
void process_wakeup(wait_queue_t *wq);
