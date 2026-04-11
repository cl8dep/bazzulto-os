#include "../library/systemcall.h"
#include "string.h"

int main(int argc, char *argv[])
{
	// Print arguments separated by spaces, followed by a newline.
	// argv[0] is the program name ("echo") — skip it.
	for (int i = 1; i < argc; i++) {
		if (i > 1)
			write(1, " ", 1);
		write(1, argv[i], strlen(argv[i]));
	}
	write(1, "\r\n", 2);
	return 0;
}
