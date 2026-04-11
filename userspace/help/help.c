#include "../library/systemcall.h"
#include "string.h"
#include "stdio.h"

#define NAME_BUF_SIZE 64

int main(void)
{
	printf("Bazzulto OS — available commands:\r\n\r\n");

	char name[NAME_BUF_SIZE];
	for (int i = 0; ; i++) {
		int64_t size = list(i, name, NAME_BUF_SIZE);
		if (size < 0)
			break;

		// Only show entries under /bin/ and skip the "/bin/" prefix for display.
		if (name[0] == '/' && name[1] == 'b' && name[2] == 'i' &&
		    name[3] == 'n' && name[4] == '/') {
			printf("  %s\r\n", name + 5);
		}
	}

	printf("\r\nType a command name to run it.\r\n");
	return 0;
}
