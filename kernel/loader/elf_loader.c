#include "../../include/bazzulto/elf_loader.h"
#include "../../include/bazzulto/elf.h"
#include "../../include/bazzulto/virtual_memory.h"
#include "../../include/bazzulto/physical_memory.h"
#include "../../include/bazzulto/kernel.h"
#include "../../include/bazzulto/uart.h"

// User stack: mapped below this address.
#define USER_STACK_TOP   0x7FFFFFF000ULL
#define USER_STACK_PAGES 4  // 16KB stack — enough for C programs with moderate locals

// --- Helpers (no libc) ---

static void memory_copy(void *dst, const void *src, size_t n)
{
	uint8_t *d = dst;
	const uint8_t *s = src;
	for (size_t i = 0; i < n; i++)
		d[i] = s[i];
}

static void memory_zero(void *dst, size_t n)
{
	uint8_t *d = dst;
	for (size_t i = 0; i < n; i++)
		d[i] = 0;
}

static size_t string_length(const char *s)
{
	size_t len = 0;
	while (s[len])
		len++;
	return len;
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
		memory_zero(page_virt, PAGE_SIZE);

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
			memory_copy(page_virt + dst_offset, file_data + src_offset, copy_size);
		}
		// Bytes outside [p_vaddr, p_vaddr + p_filesz) but within
		// [p_vaddr, p_vaddr + p_memsz) are already zeroed (the .bss region).

		virtual_memory_map(page_table, vaddr, (uint64_t)phys, page_flags);
	}

	return 0;
}

// --- Public API ---

process_t *elf_loader_load(const void *data, size_t size,
                            const char *const *argv, int argc)
{
	const uint8_t *file_data = (const uint8_t *)data;

	// The file must be large enough to contain the ELF header.
	if (size < sizeof(elf64_header_t)) {
		uart_puts("[elf_loader] file too small\n");
		return NULL;
	}

	const elf64_header_t *header = (const elf64_header_t *)file_data;

	if (validate_elf_header(header, size) < 0)
		return NULL;

	if (header->e_phnum == 0) {
		uart_puts("[elf_loader] no program headers\n");
		return NULL;
	}

	// Create a per-process page table.
	uint64_t *page_table = virtual_memory_create_table();
	if (!page_table) {
		uart_puts("[elf_loader] page table allocation failed\n");
		return NULL;
	}

	// Iterate program headers and map all PT_LOAD segments.
	const uint8_t *phdr_base = file_data + header->e_phoff;
	for (uint16_t i = 0; i < header->e_phnum; i++) {
		const elf64_program_header_t *phdr =
			(const elf64_program_header_t *)(phdr_base + i * header->e_phentsize);

		if (phdr->p_type != ELF_PROGRAM_TYPE_LOAD)
			continue;

		if (phdr->p_offset + phdr->p_filesz > size) {
			uart_puts("[elf_loader] segment file data out of bounds\n");
			return NULL;
		}

		if (phdr->p_memsz < phdr->p_filesz) {
			uart_puts("[elf_loader] p_memsz < p_filesz\n");
			return NULL;
		}

		if (phdr->p_vaddr + phdr->p_memsz > 0x0001000000000000ULL) {
			uart_puts("[elf_loader] segment vaddr out of user range\n");
			return NULL;
		}

		if (map_segment(page_table, file_data, phdr) < 0) {
			uart_puts("[elf_loader] segment mapping failed\n");
			return NULL;
		}
	}

	// --- Allocate user stack pages ---
	// Keep a reference to the top page so we can write argv data via HHDM.
	void *top_page_phys = NULL;
	for (int i = 0; i < USER_STACK_PAGES; i++) {
		void *stack_phys = physical_memory_alloc();
		if (!stack_phys) {
			uart_puts("[elf_loader] stack allocation failed\n");
			return NULL;
		}
		memory_zero(PHYSICAL_TO_VIRTUAL(stack_phys), PAGE_SIZE);
		uint64_t stack_vaddr = USER_STACK_TOP - (uint64_t)(i + 1) * PAGE_SIZE;
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
	// The top stack page covers [USER_STACK_TOP - PAGE_SIZE, USER_STACK_TOP).
	// We write via HHDM: user addr X maps to top_page_virt + (X - page_base).

	uint8_t *top_page_virt = (uint8_t *)PHYSICAL_TO_VIRTUAL(top_page_phys);
	uint64_t page_base = USER_STACK_TOP - PAGE_SIZE;
	uint64_t user_sp = USER_STACK_TOP;

	if (argv && argc > 0) {
		// 1. Copy strings to top of stack, growing downward.
		uint64_t str_user_addrs[64]; // user-space addresses of each string
		for (int i = argc - 1; i >= 0; i--) {
			size_t len = string_length(argv[i]) + 1; // include '\0'
			user_sp -= len;
			memory_copy(top_page_virt + (user_sp - page_base), argv[i], len);
			str_user_addrs[i] = user_sp;
		}

		// 2. Align down to 8 bytes for the pointer array.
		user_sp &= ~(uint64_t)7;

		// 3. Write NULL terminator for argv array.
		user_sp -= 8;
		uint64_t *slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
		*slot = 0;

		// 4. Write argv pointers (in reverse so argv[0] is at lowest address).
		for (int i = argc - 1; i >= 0; i--) {
			user_sp -= 8;
			slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
			*slot = str_user_addrs[i];
		}
	} else {
		// No arguments: write argv[0] = NULL.
		user_sp &= ~(uint64_t)7;
		user_sp -= 8;
		uint64_t *slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
		*slot = 0;
	}

	// 5. Write argc.
	user_sp -= 8;
	uint64_t *argc_slot = (uint64_t *)(top_page_virt + (user_sp - page_base));
	*argc_slot = (uint64_t)argc;

	// 6. Align SP to 16 bytes (AAPCS64 requirement).
	user_sp &= ~(uint64_t)15;

	// Create the process with the prepared page table and adjusted SP.
	process_t *process = scheduler_create_user_process_from_image(
		page_table, header->e_entry, user_sp);

	if (!process) {
		uart_puts("[elf_loader] process creation failed\n");
		return NULL;
	}

	return process;
}
