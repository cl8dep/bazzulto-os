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

// Print an integer as decimal.
static void print_number(int64_t n)
{
	if (n < 0) {
		write(1, "-", 1);
		n = -n;
	}
	char digits[20];
	int len = 0;
	if (n == 0) {
		digits[len++] = '0';
	} else {
		while (n > 0) {
			digits[len++] = '0' + (int)(n % 10);
			n /= 10;
		}
	}
	// Reverse.
	for (int i = 0, j = len - 1; i < j; i++, j--) {
		char tmp = digits[i];
		digits[i] = digits[j];
		digits[j] = tmp;
	}
	write(1, digits, (size_t)len);
}

int main(void)
{
	char name[NAME_BUF_SIZE];

	for (int i = 0; ; i++) {
		int64_t size = list(i, name, NAME_BUF_SIZE);
		if (size < 0)
			break;

		print(name);
		print("  (");
		print_number(size);
		print(" bytes)\r\n");
	}

	return 0;
}
