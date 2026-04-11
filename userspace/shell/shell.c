#include "../library/systemcall.h"

#define MAX_INPUT 128
#define MAX_ARGS  16

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

// Read a line from stdin into `buf`. Returns the number of characters read.
// Echoes each character back and handles backspace.
static size_t read_line(char *buf, size_t max)
{
	size_t pos = 0;
	while (pos < max - 1) {
		char c;
		int64_t n = read(0, &c, 1);
		if (n <= 0)
			continue;

		if (c == '\r' || c == '\n') {
			print("\r\n");
			break;
		}

		// Backspace (0x7F) or Ctrl-H (0x08)
		if (c == 0x7F || c == 0x08) {
			if (pos > 0) {
				pos--;
				print("\b \b");
			}
			continue;
		}

		// Ignore non-printable characters.
		if (c < 0x20)
			continue;

		buf[pos++] = c;
		write(1, &c, 1);
	}
	buf[pos] = '\0';
	return pos;
}

// Split `input` into tokens by spaces. Modifies `input` in place
// (replaces spaces with '\0'). Fills `argv` with pointers to each token.
// Returns argc (number of tokens).
static int tokenize(char *input, const char **argv, int max_args)
{
	int argc = 0;

	while (*input && argc < max_args) {
		// Skip leading spaces.
		while (*input == ' ')
			input++;
		if (*input == '\0')
			break;

		// Start of a token.
		argv[argc++] = input;

		// Find end of token.
		while (*input && *input != ' ')
			input++;

		// Null-terminate the token.
		if (*input) {
			*input = '\0';
			input++;
		}
	}

	return argc;
}

// Copy src into dst, prepending "/bin/". Returns total length.
static size_t build_path(char *dst, size_t dst_size, const char *cmd)
{
	const char *prefix = "/bin/";
	size_t i = 0;

	while (*prefix && i < dst_size - 1)
		dst[i++] = *prefix++;
	while (*cmd && i < dst_size - 1)
		dst[i++] = *cmd++;
	dst[i] = '\0';
	return i;
}

int main(void)
{
	char input[MAX_INPUT];
	char path[MAX_INPUT];
	const char *argv[MAX_ARGS + 1]; // +1 for NULL terminator

	print("Bazzulto OS shell\r\n");

	for (;;) {
		print("bazzulto> ");
		size_t len = read_line(input, MAX_INPUT);

		if (len == 0)
			continue;

		// Split input into tokens.
		int argc = tokenize(input, argv, MAX_ARGS);
		if (argc == 0)
			continue;

		// Build /bin/<command> path from the first token.
		build_path(path, MAX_INPUT, argv[0]);

		// NULL-terminate the argv array.
		argv[argc] = (const char *)0;

		// Try to spawn the program with arguments.
		int pid = spawn(path, argv);
		if (pid < 0) {
			print(argv[0]);
			print(": command not found\r\n");
			continue;
		}

		// Block until the child exits so its output lands before the next prompt.
		wait(pid);
	}
}
