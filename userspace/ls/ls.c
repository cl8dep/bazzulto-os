#include "../library/systemcall.h"
#include "string.h"
#include "stdio.h"

#define NAME_BUF_SIZE 64

int main(void)
{
	char name[NAME_BUF_SIZE];

	for (int i = 0; ; i++) {
		int64_t size = list(i, name, NAME_BUF_SIZE);
		if (size < 0)
			break;

		printf("%s  (%ld bytes)\r\n", name, (long)size);
	}

	return 0;
}
