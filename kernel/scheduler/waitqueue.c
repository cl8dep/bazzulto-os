#include "../../include/bazzulto/waitqueue.h"

void process_sleep(wait_queue_t *wq) {
	process_t *p = scheduler_get_current();
	p->state = PROCESS_STATE_BLOCKED;

	// Append to the tail of the wait queue.
	p->wait_next = NULL;
	if (!wq->head) {
		wq->head = p;
	} else {
		process_t *tail = wq->head;
		while (tail->wait_next)
			tail = tail->wait_next;
		tail->wait_next = p;
	}

	// Yield to the next ready process. When this process is woken up
	// and switched back to, scheduler_yield returns here.
	scheduler_yield();
}

void process_wakeup(wait_queue_t *wq) {
	if (!wq->head)
		return;

	// Remove the first waiter (FIFO) and mark it ready.
	process_t *p = wq->head;
	wq->head = p->wait_next;
	p->wait_next = NULL;
	p->state = PROCESS_STATE_READY;
}
