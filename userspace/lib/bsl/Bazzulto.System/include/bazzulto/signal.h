#pragma once
/**
 * @file signal.h
 * @brief Bazzulto.System — signal C ABI.
 *
 * Provides:
 *   - Signal number constants (POSIX-compatible).
 *   - sigaction() for installing per-signal handlers.
 *   - kill() for sending signals to processes.
 *   - sigprocmask() for blocking / unblocking signals.
 *   - sigaltstack() for designating an alternate signal stack.
 *   - sigpending() / sigsuspend() for advanced signal management.
 *
 * Signal numbers match their POSIX values so that existing code and tooling
 * remain compatible.  Signals 1–31 follow the Linux/POSIX numbering.
 */

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------
 * Signal numbers
 * ------------------------------------------------------------------------- */

/** @defgroup signal_numbers Signal Numbers
 *  @{
 */
#define BZ_SIGHUP    1   /**< Hangup (controlling terminal closed). */
#define BZ_SIGINT    2   /**< Terminal interrupt (Ctrl+C). */
#define BZ_SIGQUIT   3   /**< Terminal quit (Ctrl+\\). */
#define BZ_SIGILL    4   /**< Illegal instruction. */
#define BZ_SIGTRAP   5   /**< Trace / breakpoint trap. */
#define BZ_SIGABRT   6   /**< Process abort (abort()). */
#define BZ_SIGFPE    8   /**< Floating-point / arithmetic exception. */
#define BZ_SIGKILL   9   /**< Kill — cannot be caught or ignored. */
#define BZ_SIGUSR1  10   /**< User-defined signal 1. */
#define BZ_SIGSEGV  11   /**< Invalid memory reference. */
#define BZ_SIGUSR2  12   /**< User-defined signal 2. */
#define BZ_SIGPIPE  13   /**< Write to a pipe with no readers. */
#define BZ_SIGALRM  14   /**< Alarm clock (alarm()). */
#define BZ_SIGTERM  15   /**< Graceful termination request. */
#define BZ_SIGCHLD  17   /**< Child process stopped or terminated. */
#define BZ_SIGCONT  18   /**< Continue a stopped process. */
#define BZ_SIGSTOP  19   /**< Stop — cannot be caught or ignored. */
#define BZ_SIGTSTP  20   /**< Terminal stop (Ctrl+Z). */
#define BZ_SIGTTIN  21   /**< Background process attempted read from terminal. */
#define BZ_SIGTTOU  22   /**< Background process attempted write to terminal. */
#define BZ_SIGWINCH 28   /**< Terminal window size changed. */
/** @} */

/* ---------------------------------------------------------------------------
 * Special handler values
 * ------------------------------------------------------------------------- */

/** @defgroup signal_handlers Special Handler Values
 *  @{
 */
#define BZ_SIG_DFL  ((uint64_t)0)  /**< Default signal action. */
#define BZ_SIG_IGN  ((uint64_t)1)  /**< Ignore the signal. */
/** @} */

/* ---------------------------------------------------------------------------
 * sigaction flags
 * ------------------------------------------------------------------------- */

/** @defgroup sigaction_flags sigaction() Flags
 *  @{
 */
/** Deliver the signal on the alternate signal stack (see bz_sigaltstack()). */
#define BZ_SA_ONSTACK   0x08000000u
/** Restart interrupted system calls automatically. */
#define BZ_SA_RESTART   0x10000000u
/** Reset the handler to BZ_SIG_DFL after the first delivery. */
#define BZ_SA_RESETHAND 0x80000000u
/** Do not send SIGCHLD when a child stops (only when it terminates). */
#define BZ_SA_NOCLDSTOP 0x00000001u
/** @} */

/* ---------------------------------------------------------------------------
 * sigprocmask() how values
 * ------------------------------------------------------------------------- */

/** @defgroup sigprocmask_how sigprocmask() How Values
 *  @{
 */
#define BZ_SIG_BLOCK    0  /**< Add @p set to the current signal mask. */
#define BZ_SIG_UNBLOCK  1  /**< Remove @p set from the current signal mask. */
#define BZ_SIG_SETMASK  2  /**< Replace the current signal mask with @p set. */
/** @} */

/* ---------------------------------------------------------------------------
 * sigaltstack — alternate signal stack
 * ------------------------------------------------------------------------- */

/** @defgroup sigaltstack_flags sigaltstack() Flags
 *  @{
 */
/** Disable the alternate signal stack. */
#define BZ_SS_DISABLE   4
/** Set when the process is currently executing on the alternate signal stack
 *  (returned in @c ss_flags by bz_sigaltstack(); never passed as input). */
#define BZ_SS_ONSTACK   1
/** @} */

/**
 * @brief Alternate signal stack descriptor.
 *
 * Analogous to POSIX @c stack_t.  Pass to bz_sigaltstack() to install an
 * alternate stack so that signal handlers marked @c BZ_SA_ONSTACK are
 * delivered there instead of the normal process stack.
 *
 * @note The alternate stack must be large enough for the handler's frame.
 *       The minimum useful size is typically 8 KiB; 64 KiB is recommended.
 */
typedef struct {
    void    *ss_sp;     /**< Base address of the alternate stack. */
    int32_t  ss_flags;  /**< BZ_SS_DISABLE or 0. */
    size_t   ss_size;   /**< Size of the alternate stack in bytes. */
} bz_stack_t;

/* ---------------------------------------------------------------------------
 * Functions
 * ------------------------------------------------------------------------- */

/**
 * @brief Install or query a signal handler.
 *
 * @param signal_number   Signal to configure (one of the @c BZ_SIG* constants).
 * @param handler_va      New handler: @c BZ_SIG_DFL, @c BZ_SIG_IGN, or the
 *                        virtual address of a @c void(*)(int) function.
 * @param flags           Combination of @c BZ_SA_* flags, or 0.
 * @param old_handler_out Written with the previous handler value. May be NULL.
 * @return 0 on success, or a negative errno value on failure.
 */
int64_t bz_sigaction(int32_t signal_number, uint64_t handler_va,
                     uint32_t flags, uint64_t *old_handler_out);

/**
 * @brief Send a signal to a process.
 *
 * @param pid           Target process PID (negative values address process
 *                      groups: -pgid sends to every process in that group).
 * @param signal_number Signal to send (one of the @c BZ_SIG* constants).
 * @return 0 on success, or a negative errno value on failure.
 */
int64_t bz_kill(int32_t pid, int32_t signal_number);

/**
 * @brief Examine and/or change the set of blocked signals.
 *
 * @p set is a 64-bit bitmask where bit @c N corresponds to signal @c N+1
 * (bit 0 = SIG 1 = SIGHUP, bit 1 = SIG 2 = SIGINT, …).
 *
 * @param how     @c BZ_SIG_BLOCK, @c BZ_SIG_UNBLOCK, or @c BZ_SIG_SETMASK.
 * @param set     New or delta signal set.  Ignored if @p old_set_out is the
 *                only objective — pass 0 with @c BZ_SIG_BLOCK to query.
 * @param old_set_out  Written with the previous signal mask. May be NULL.
 * @return 0 on success, or a negative errno value on failure.
 */
int64_t bz_sigprocmask(int32_t how, uint64_t set, uint64_t *old_set_out);

/**
 * @brief Return the set of signals that are pending for the calling process.
 *
 * A signal is pending if it has been delivered to the process but is currently
 * blocked by the signal mask.
 *
 * @param set_out  Written with the pending signal bitmask.
 * @return 0 on success, or a negative errno value on failure.
 */
int64_t bz_sigpending(uint64_t *set_out);

/**
 * @brief Atomically replace the signal mask and wait for a signal.
 *
 * Equivalent to POSIX sigsuspend().  Replaces the signal mask with @p mask,
 * then suspends the process until a signal is delivered.  The original mask is
 * restored before bz_sigsuspend() returns.
 *
 * @param mask  Signal mask to apply while suspended.
 * @return Always returns -BZ_EINTR (a signal interrupted the wait).
 */
int64_t bz_sigsuspend(uint64_t mask);

/**
 * @brief Query or change the alternate signal stack.
 *
 * When an alternate stack is installed and a handler is registered with
 * @c BZ_SA_ONSTACK, the kernel switches the stack pointer to the alternate
 * stack before calling the handler.  This prevents stack overflow from
 * corrupting the main stack during @c SIGSEGV handling.
 *
 * @param new_stack  New alternate stack descriptor, or NULL to query only.
 *                   Set @c ss_flags to @c BZ_SS_DISABLE to disable.
 * @param old_stack  Written with the current alternate stack state. May be NULL.
 * @return 0 on success, or a negative errno value on failure.
 *         Returns @c -BZ_EPERM if called while executing on the alternate stack.
 */
int64_t bz_sigaltstack(const bz_stack_t *new_stack, bz_stack_t *old_stack);

/**
 * @brief Return from a signal handler.
 *
 * Called automatically by the vDSO signal trampoline at 0x2000.  User code
 * should not call this directly — it is invoked when a signal handler function
 * returns normally.
 *
 * @return Does not return (restores pre-signal CPU state and resumes execution).
 */
int64_t bz_sigreturn(void);

#ifdef __cplusplus
} /* extern "C" */
#endif
