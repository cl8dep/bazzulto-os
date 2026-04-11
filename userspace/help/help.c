#include "../library/systemcall.h"

#define NAME_BUF_SIZE 64

static size_t string_length(const char *s)
{
	size_t len = 0;
	while (s[len])
		len++;
	return len;
}

static void print(const char *s)
{
	write(1, s, string_length(s));
}

int main(void)
{
	print("Bazzulto OS — available commands:\r\n");
	print("\r\n");

	char name[NAME_BUF_SIZE];
	for (int i = 0; ; i++) {
		int64_t size = list(i, name, NAME_BUF_SIZE);
		if (size < 0)
			break;

		// Only show entries under /bin/ and skip "/bin/" prefix for display.
		const char *p = name;
		// Check if name starts with "/bin/"
		if (p[0] == '/' && p[1] == 'b' && p[2] == 'i' && p[3] == 'n' && p[4] == '/') {
			print("  ");
			print(p + 5); // skip "/bin/"
			print("\r\n");
		}
	}

	print("\r\nType a command name to run it.\r\n");
	return 0;
}
