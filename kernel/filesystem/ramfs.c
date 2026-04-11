#include "../../include/bazzulto/ramfs.h"
#include "../../include/bazzulto/console.h"
#include <string.h>

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


int ramfs_register(const char *name, const void *data, size_t size)
{
	if (file_count >= RAMFS_MAX_FILES)
		return -1;
	if (strlen(name) >= RAMFS_MAX_NAME)
		return -1;

	struct ramfs_file *f = &file_table[file_count];
	strcpy(f->name, name);
	f->data = (const uint8_t *)data;
	f->size = size;
	file_count++;
	return 0;
}

const struct ramfs_file *ramfs_lookup(const char *name)
{
	for (int i = 0; i < file_count; i++) {
		if (strcmp(file_table[i].name, name) == 0)
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
