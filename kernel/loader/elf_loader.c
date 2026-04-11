#include "../../include/bazzulto/elf_loader.h"
#include <string.h>
#include "../../include/bazzulto/elf.h"
#include "../../include/bazzulto/virtual_memory.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/kernel.h"
#include "../../include/bazzulto/uart.h"

// Signal return trampoline — mapped read+execute at this VA in every user process.
// Contains a single `svc #SYSTEMCALL_SIGRETURN` instruction (number 23).
// ARM ARM C6.2.326: svc #N encoding = 0xD4000001 | (N << 5).
//   SYSTEMCALL_SIGRETURN = 23 → svc #23 = 0xD4000001 | (23 << 5) = 0xD40002E1.
// When a signal handler returns (via LR = SIGNAL_TRAMPOLINE_VA), the CPU
// executes this instruction which invokes sys_sigreturn to restore context.
#define SIGNAL_TRAMPOLINE_VA    0x1000ULL
#define SIGNAL_TRAMPOLINE_INSN  0xD40002E1U  // svc #23

// User stack: mapped below this base address.
// The actual stack top is varied per process by ASLR (see aslr_stack_offset()).
#define USER_STACK_BASE  0x7FFFF00000ULL
#define USER_STACK_PAGES 4  // 16KB — enough for C programs with moderate call depth

// Guard page: the PAGE_SIZE region immediately below the bottom stack page is
// intentionally left unmapped. Any stack overflow will fault here before
// touching the heap or other data. No extra code needed — the guard is implicit
// because we only map USER_STACK_PAGES pages above the guard address.
#define USER_STACK_GUARD_PAGES 1

// ---------------------------------------------------------------------------
// Stack ASLR — ARM ARM D13.2.14: CNTPCT_EL0 is readable from EL1 and above.
// We mix the timer counter with the page table pointer to get per-process
// entropy. This randomizes the stack position by 0–255 pages (0–1020 KB)
// on top of USER_STACK_BASE. Full text-segment ASLR requires PIE compilation.
// ---------------------------------------------------------------------------
static uint64_t aslr_stack_offset(const void *page_table_seed)
{
    uint64_t count;
    __asm__ volatile("mrs %0, cntpct_el0" : "=r"(count));
    // Mix timer bits with a pointer-derived value so even processes that start
    // at the same tick get different offsets.
    uint64_t mixed = count ^ ((uint64_t)(uintptr_t)page_table_seed >> 12);
    return (mixed & 0xFF) * PAGE_SIZE;  // 0-255 pages above guard
}

// --- ELF validation ---

// Verify the ELF header is a valid AArch64 executable.
// Returns 0 on success, -1 on failure.
static int validate_elf_header(const elf64_header_t *header, size_t file_size)
{
	// Check magic bytes: 0x7F 'E' 'L' 'F'
	if (header->e_ident[ELF_IDENT_MAGIC0] != 0x7F ||
	    header->e_ident[ELF_IDENT_MAGIC1] != 'E'  ||
	    header->e_ident[ELF_IDENT_MAGIC2] != 'L'  ||
	    header->e_ident[ELF_IDENT_MAGIC3] != 'F') {
		uart_puts("[elf_loader] invalid magic\n");
		return -1;
	}

	// Must be 64-bit ELF
	if (header->e_ident[ELF_IDENT_CLASS] != ELF_CLASS_64) {
		uart_puts("[elf_loader] not ELF-64\n");
		return -1;
	}

	// Must be little-endian (AArch64)
	if (header->e_ident[ELF_IDENT_DATA] != ELF_DATA_LITTLE_ENDIAN) {
		uart_puts("[elf_loader] not little-endian\n");
		return -1;
	}

	// Must be an executable (not shared object, relocatable, or core)
	if (header->e_type != ELF_TYPE_EXECUTABLE) {
		uart_puts("[elf_loader] not an executable\n");
		return -1;
	}

	// Must target AArch64
	if (header->e_machine != ELF_MACHINE_AARCH64) {
		uart_puts("[elf_loader] not AArch64\n");
		return -1;
	}

	// Program header table must fit within the file
	if (header->e_phoff + (uint64_t)header->e_phnum * header->e_phentsize > file_size) {
		uart_puts("[elf_loader] program headers out of bounds\n");
		return -1;
	}

	return 0;
}

// --- Segment mapping ---

// Convert ELF p_flags to page table flags.
// ARM ARM DDI 0487 D5.4.4: AP and XN bits control permissions.
static uint64_t elf_flags_to_page_flags(uint32_t p_flags)
{
	// Base flags for any user-space mapping: valid, L3 page, access flag,
	// normal cacheable memory, inner shareable.
	uint64_t flags = PAGE_VALID | PAGE_TABLE | PAGE_ACCESS_FLAG |
	                 PAGE_ATTR_NORMAL | PAGE_SH_INNER;

	if (p_flags & ELF_PROGRAM_FLAG_WRITE) {
		// Writable segment (data, bss, stack).
		// AP[2:1]=01: EL1+EL0 read/write.
		flags |= PAGE_USER_RW;
		// Writable pages should not be executable (W^X).
		flags |= PAGE_PXN | PAGE_UXN;
	} else {
		// Read-only segment.
		// AP[2:1]=11: EL1+EL0 read-only.
		flags |= PAGE_USER_RO;
		if (p_flags & ELF_PROGRAM_FLAG_EXECUTE) {
			// Executable: allow EL0 execution, prevent EL1 execution.
			flags |= PAGE_PXN;
			// UXN is NOT set — EL0 can execute.
		} else {
			// Not executable: prevent execution from both EL0 and EL1.
			flags |= PAGE_PXN | PAGE_UXN;
		}
	}

	return flags;
}

// Map one PT_LOAD segment into the process page table.
// Allocates physical pages, copies file data, zeros the .bss portion.
static int map_segment(uint64_t *page_table,
                        const uint8_t *file_data,
                        const elf64_program_header_t *phdr)
{
	uint64_t vaddr_start = phdr->p_vaddr & ~(uint64_t)(PAGE_SIZE - 1);
	uint64_t vaddr_end   = (phdr->p_vaddr + phdr->p_memsz + PAGE_SIZE - 1) &
	                        ~(uint64_t)(PAGE_SIZE - 1);
	uint64_t page_flags  = elf_flags_to_page_flags(phdr->p_flags);

	for (uint64_t vaddr = vaddr_start; vaddr < vaddr_end; vaddr += PAGE_SIZE) {
		void *phys = physical_memory_alloc();
		if (!phys)
			return -1;

		uint8_t *page_virt = (uint8_t *)PHYSICAL_TO_VIRTUAL(phys);
		memset(page_virt, 0, PAGE_SIZE);

		// Calculate which bytes of this page overlap with the file data
		// region [p_vaddr, p_vaddr + p_filesz).
		uint64_t seg_file_start = phdr->p_vaddr;
		uint64_t seg_file_end   = phdr->p_vaddr + phdr->p_filesz;

		uint64_t page_start = vaddr;
		uint64_t page_end   = vaddr + PAGE_SIZE;

		// Overlap region between this page and the file data portion.
		uint64_t copy_start = (page_start > seg_file_start) ? page_start : seg_file_start;
		uint64_t copy_end   = (page_end < seg_file_end) ? page_end : seg_file_end;

		if (copy_start < copy_end) {
			size_t dst_offset  = copy_start - page_start;
			size_t src_offset  = phdr->p_offset + (copy_start - seg_file_start);
			size_t copy_size   = copy_end - copy_start;
			memcpy(page_virt + dst_offset, file_data + src_offset, copy_size);
		}
		// Bytes outside [p_vaddr, p_vaddr + p_filesz) but within
		// [p_vaddr, p_vaddr + p_memsz) are already zeroed (the .bss region).

		virtual_memory_map(page_table, vaddr, (uint64_t)phys, page_flags);
	}

	return 0;
}

// --- Public API ---

// Build an ELF image: validate, map segments, allocate stack, push argv.
// Does NOT create a scheduler process — just returns the page table, entry
// point, and user SP. This is the shared core used by both elf_loader_load
// (spawn) and sys_exec (replace current process image).
int elf_loader_build_image(const void *data, size_t size,
                            const char *const *argv, int argc,
                            uint64_t **page_table_out,
                            uint64_t  *entry_out,
                            uint64_t  *stack_top_out)
{
	const uint8_t *file_data = (const uint8_t *)data;

	if (size < sizeof(elf64_header_t)) {
		uart_puts("[elf_loader] file too small\n");
		return -1;
	}

	const elf64_header_t *header = (const elf64_header_t *)file_data;

	if (validate_elf_header(header, size) < 0)
		return -1;

	if (header->e_phnum == 0) {
		uart_puts("[elf_loader] no program headers\n");
		return -1;
	}

	uint64_t *page_table = virtual_memory_create_table();
	if (!page_table) {
		uart_puts("[elf_loader] page table allocation failed\n");
		return -1;
	}

	const uint8_t *phdr_base = file_data + header->e_phoff;
	for (uint16_t i = 0; i < header->e_phnum; i++) {
		const elf64_program_header_t *phdr =
			(const elf64_program_header_t *)(phdr_base + i * header->e_phentsize);

		if (phdr->p_type != ELF_PROGRAM_TYPE_LOAD)
			continue;

		if (phdr->p_offset + phdr->p_filesz > size) {
			uart_puts("[elf_loader] segment file data out of bounds\n");
			return -1;
		}

		if (phdr->p_memsz < phdr->p_filesz) {
			uart_puts("[elf_loader] p_memsz < p_filesz\n");
			return -1;
		}

		if (phdr->p_vaddr + phdr->p_memsz > 0x0001000000000000ULL) {
			uart_puts("[elf_loader] segment vaddr out of user range\n");
			return -1;
		}

		if (map_segment(page_table, file_data, phdr) < 0) {
			uart_puts("[elf_loader] segment mapping failed\n");
			return -1;
		}
	}

	uint64_t stack_top = USER_STACK_BASE - aslr_stack_offset(page_table);

	void *top_page_phys = NULL;
	for (int i = 0; i < USER_STACK_PAGES; i++) {
		void *stack_phys = physical_memory_alloc();
		if (!stack_phys) {
			uart_puts("[elf_loader] stack allocation failed\n");
			return -1;
		}
		memset(PHYSICAL_TO_VIRTUAL(stack_phys), 0, PAGE_SIZE);
		uint64_t stack_vaddr = stack_top - (uint64_t)(i + 1) * PAGE_SIZE;
		virtual_memory_map(page_table, stack_vaddr,
		                   (uint64_t)stack_phys, PAGE_FLAGS_USER_DATA);
		if (i == 0)
			top_page_phys = stack_phys;
	}

	// --- Set up argv on the user stack ---
	//
	// Stack layout (addresses decrease downward):
	//
	//   USER_STACK_TOP
	//   ├── argv string data ("echo\0hola\0mundo\0")
	//   ├── padding to 8-byte alignment
	//   ├── argv[argc] = NULL
	//   ├── argv[argc-1] → string
	//   ├── ...
	//   ├── argv[0] → string
	//   ├── argc (uint64_t)
	//   └── SP (16-byte aligned per AAPCS64)
	//
	// The top stack page covers [stack_top - PAGE_SIZE, stack_top).
	// We write via HHDM: user addr X maps to top_page_virt + (X - page_base).

	uint8_t *top_page_virt = (uint8_t *)PHYSICAL_TO_VIRTUAL(top_page_phys);
	uint64_t page_base = stack_top - PAGE_SIZE;
	uint64_t user_sp = stack_top;

	if (argv && argc > 0) {
		uint64_t str_user_addrs[64];
		for (int i = argc - 1; i >= 0; i--) {
			size_t len = strlen(argv[i]) + 1;
			user_sp -= len;
			memcpy(top_page_virt + (user_sp - page_base), argv[i], len);
			str_user_addrs[i] = user_sp;
		}

		user_sp &= ~(uint64_t)7;

		user_sp -= 8;
		uint64_t *slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
		*slot = 0;

		for (int i = argc - 1; i >= 0; i--) {
			user_sp -= 8;
			slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
			*slot = str_user_addrs[i];
		}
	} else {
		user_sp &= ~(uint64_t)7;
		user_sp -= 8;
		uint64_t *slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
		*slot = 0;
	}

	user_sp -= 8;
	uint64_t *argc_slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
	*argc_slot = (uint64_t)argc;

	user_sp &= ~(uint64_t)15;

	// --- Map the signal return trampoline ---
	//
	// One read+execute page at SIGNAL_TRAMPOLINE_VA (0x1000), safely below
	// the ELF load address (0x400000). The page contains a single instruction:
	//   svc #SYSTEMCALL_SIGRETURN
	// Signal handlers return via this address (placed in LR at delivery time).
	void *trampoline_phys = physical_memory_alloc();
	if (!trampoline_phys) {
		uart_puts("[elf_loader] trampoline allocation failed\n");
		return -1;
	}
	memset(PHYSICAL_TO_VIRTUAL(trampoline_phys), 0, PAGE_SIZE);
	uint32_t *trampoline_code = (uint32_t *)PHYSICAL_TO_VIRTUAL(trampoline_phys);
	trampoline_code[0] = SIGNAL_TRAMPOLINE_INSN;
	virtual_memory_map(page_table, SIGNAL_TRAMPOLINE_VA,
	                   (uint64_t)trampoline_phys, PAGE_FLAGS_USER_CODE);

	*page_table_out = page_table;
	*entry_out      = header->e_entry;
	*stack_top_out  = user_sp;
	return 0;
}

process_t *elf_loader_load(const void *data, size_t size,
                            const char *const *argv, int argc)
{
	uint64_t *page_table = NULL;
	uint64_t  entry      = 0;
	uint64_t  user_sp    = 0;

	if (elf_loader_build_image(data, size, argv, argc,
	                            &page_table, &entry, &user_sp) < 0)
		return NULL;

	process_t *process = scheduler_create_user_process_from_image(
		page_table, entry, user_sp);

	if (!process) {
		uart_puts("[elf_loader] process creation failed\n");
		return NULL;
	}

	return process;
}
