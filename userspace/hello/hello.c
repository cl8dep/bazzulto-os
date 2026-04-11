#include "../library/systemcall.h"

int main(void)
{
	const char *msg = "Hello from ELF!\n";

	// Count string length.
	size_t len = 0;
	while (msg[len])
		len++;

	write(1, msg, len);
	return 0;
}
