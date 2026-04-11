#include "../../include/bazzulto/ramfs.h"
#include "../../include/bazzulto/console.h"

// File table — populated at boot, read-only thereafter.
static struct ramfs_file file_table[RAMFS_MAX_FILES];
static int file_count;

void ramfs_init(void)
{
	file_count = 0;
	for (int i = 0; i < RAMFS_MAX_FILES; i++) {
		file_table[i].name[0] = '\0';
		file_table[i].data = NULL;
		file_table[i].size = 0;
	}
	console_println("ramfs: ok");
}

// Simple strlen — no libc available.
static size_t ramfs_strlen(const char *s)
{
	size_t len = 0;
	while (s[len])
		len++;
	return len;
}

// Simple strcpy — no libc available.
static void ramfs_strcpy(char *dst, const char *src)
{
	while (*src)
		*dst++ = *src++;
	*dst = '\0';
}

// Simple strcmp — no libc available.
static int ramfs_strcmp(const char *a, const char *b)
{
	while (*a && *a == *b) {
		a++;
		b++;
	}
	return (unsigned char)*a - (unsigned char)*b;
}

int ramfs_register(const char *name, const void *data, size_t size)
{
	if (file_count >= RAMFS_MAX_FILES)
		return -1;
	if (ramfs_strlen(name) >= RAMFS_MAX_NAME)
		return -1;

	struct ramfs_file *f = &file_table[file_count];
	ramfs_strcpy(f->name, name);
	f->data = (const uint8_t *)data;
	f->size = size;
	file_count++;
	return 0;
}

const struct ramfs_file *ramfs_lookup(const char *name)
{
	for (int i = 0; i < file_count; i++) {
		if (ramfs_strcmp(file_table[i].name, name) == 0)
			return &file_table[i];
	}
	return NULL;
}

int ramfs_file_count(void)
{
	return file_count;
}

const struct ramfs_file *ramfs_file_at(int index)
{
	if (index < 0 || index >= file_count)
		return NULL;
	return &file_table[index];
}
