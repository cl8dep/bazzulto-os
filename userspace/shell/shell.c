#include "../library/systemcall.h"
#include "string.h"

#define MAX_INPUT 128
#define MAX_ARGS  16

static void print(const char *s)
{
	write(1, s, strlen(s));
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

// Split `input` into tokens. Modifies `input` in place.
// Fills `argv` with pointers into `input` for each token.
// Returns argc on success, or -1 if an unterminated quote is found.
//
// Quote rules:
//   "..."  — spaces inside are part of the token; quotes themselves are stripped.
//   '...'  — identical behaviour (no variable interpolation in either form).
//   Nested quotes of the other type are treated as literal characters:
//     "it's" → it's    |    '"hi"' → "hi"
//   Empty quotes ("" or '') produce a token that is an empty string.
//   Unterminated quote → return -1.
//
// Implementation: dual read/write pointers into the same buffer.
// The write pointer compacts the token in place (removing quote chars and
// collapsing whitespace boundaries). Because characters are only removed —
// never inserted — the write pointer never overtakes the read pointer.
static int tokenize(char *input, const char **argv, int max_args)
{
	typedef enum {
		TOKENIZE_STATE_NORMAL,
		TOKENIZE_STATE_IN_DOUBLE_QUOTE,
		TOKENIZE_STATE_IN_SINGLE_QUOTE,
	} tokenize_state_t;

	int    argc      = 0;
	char  *read_ptr  = input;
	char  *write_ptr = input;

	while (*read_ptr && argc < max_args) {
		// Skip inter-token spaces.
		while (*read_ptr == ' ')
			read_ptr++;
		if (*read_ptr == '\0')
			break;

		// Record where the compacted token will start.
		char *token_start  = write_ptr;
		tokenize_state_t state = TOKENIZE_STATE_NORMAL;
		int in_token       = 1;

		while (*read_ptr && in_token) {
			char character = *read_ptr++;

			switch (state) {
			case TOKENIZE_STATE_NORMAL:
				if (character == ' ') {
					in_token = 0;
				} else if (character == '"') {
					state = TOKENIZE_STATE_IN_DOUBLE_QUOTE;
				} else if (character == '\'') {
					state = TOKENIZE_STATE_IN_SINGLE_QUOTE;
				} else {
					*write_ptr++ = character;
				}
				break;

			case TOKENIZE_STATE_IN_DOUBLE_QUOTE:
				if (character == '"') {
					state = TOKENIZE_STATE_NORMAL;
				} else {
					*write_ptr++ = character;  // spaces and inner '\'' kept
				}
				break;

			case TOKENIZE_STATE_IN_SINGLE_QUOTE:
				if (character == '\'') {
					state = TOKENIZE_STATE_NORMAL;
				} else {
					*write_ptr++ = character;  // spaces and inner '"' kept
				}
				break;
			}
		}

		// A quote that was opened but never closed is a hard error.
		if (state != TOKENIZE_STATE_NORMAL)
			return -1;

		// Null-terminate the compacted token and register it.
		*write_ptr++ = '\0';
		argv[argc++] = token_start;
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

		// Split input into tokens. -1 means an unterminated quote.
		int argc = tokenize(input, argv, MAX_ARGS);
		if (argc < 0) {
			print("error: unterminated string\r\n");
			continue;
		}
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
