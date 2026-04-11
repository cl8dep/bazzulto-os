#include "../library/systemcall.h"

static size_t string_length(const char *s)
{
	size_t len = 0;
	while (s[len])
		len++;
	return len;
}

int main(int argc, char *argv[])
{
	// Print arguments separated by spaces, followed by a newline.
	// argv[0] is the program name ("echo") — skip it.
	for (int i = 1; i < argc; i++) {
		if (i > 1)
			write(1, " ", 1);
		write(1, argv[i], string_length(argv[i]));
	}
	write(1, "\r\n", 2);
	return 0;
}
