/* Bazzulto OS — AArch64 syscall dispatch via vDSO.
 *
 * Instead of the Linux convention (svc #0 with syscall number in x8),
 * all syscalls go through the Bazzulto vDSO page mapped at 0x1000.
 * Each slot is 16 bytes (svc #N; ret; nop; nop).  The caller branches
 * to slot[n] with arguments in x0–x5 and receives the result in x0.
 *
 * This ensures ABI stability: if the kernel changes internal syscall
 * numbers, only the vDSO page is updated — no userspace recompilation
 * is needed.
 *
 * vDSO layout:
 *   Base:      0x1000
 *   Slot size: 16 bytes (4 instructions)
 *   Slot N:    0x1000 + N * 16
 *   Content:   svc #N; ret; nop; nop
 */

#define __SYSCALL_LL_E(x) (x)
#define __SYSCALL_LL_O(x) (x)

#define __VDSO_BASE  0x1000UL
#define __VDSO_SLOT  8UL

/* Branch-with-link to vDSO slot N.  x16 is the intra-procedure-call
 * scratch register on AArch64 (caller-saved, not preserved across calls).
 * blr saves the return address in x30 (lr); the vDSO slot does
 * "svc #N; ret" which returns here via x30. */
#define __asm_syscall(...) do { \
	register long x16 __asm__("x16") = __VDSO_BASE + (unsigned long)n * __VDSO_SLOT; \
	__asm__ __volatile__ ( "blr x16" \
	: "=r"(x0) : "r"(x16), __VA_ARGS__ : "memory", "cc", "x30"); \
	return x0; \
	} while (0)

static inline long __syscall0(long n)
{
	register long x0 __asm__("x0");
	register long x16 __asm__("x16") = __VDSO_BASE + (unsigned long)n * __VDSO_SLOT;
	__asm__ __volatile__ ("blr x16"
		: "=r"(x0) : "r"(x16) : "memory", "cc", "x30");
	return x0;
}

static inline long __syscall1(long n, long a)
{
	register long x0 __asm__("x0") = a;
	__asm_syscall("0"(x0));
}

static inline long __syscall2(long n, long a, long b)
{
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	__asm_syscall("0"(x0), "r"(x1));
}

static inline long __syscall3(long n, long a, long b, long c)
{
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	register long x2 __asm__("x2") = c;
	__asm_syscall("0"(x0), "r"(x1), "r"(x2));
}

static inline long __syscall4(long n, long a, long b, long c, long d)
{
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	register long x2 __asm__("x2") = c;
	register long x3 __asm__("x3") = d;
	__asm_syscall("0"(x0), "r"(x1), "r"(x2), "r"(x3));
}

static inline long __syscall5(long n, long a, long b, long c, long d, long e)
{
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	register long x2 __asm__("x2") = c;
	register long x3 __asm__("x3") = d;
	register long x4 __asm__("x4") = e;
	__asm_syscall("0"(x0), "r"(x1), "r"(x2), "r"(x3), "r"(x4));
}

static inline long __syscall6(long n, long a, long b, long c, long d, long e, long f)
{
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	register long x2 __asm__("x2") = c;
	register long x3 __asm__("x3") = d;
	register long x4 __asm__("x4") = e;
	register long x5 __asm__("x5") = f;
	__asm_syscall("0"(x0), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5));
}

/* Bazzulto vDSO provides clock_gettime directly — musl's vDSO lookup
 * mechanism (ELF symbol search) is not used.  Disable it. */
#undef VDSO_USEFUL

#define IPC_64 0
