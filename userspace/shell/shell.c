#include "../library/systemcall.h"
#include "string.h"

#define MAX_INPUT           256
#define MAX_ARGS            16
#define MAX_PIPELINE_STAGES 8

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tokenizer: splits `input` on whitespace, handling single and double quotes.
// Modifies `input` in place. Returns argc, or -1 on unterminated quote.
//
// Quote rules:
//   "..."  — spaces inside are part of the token; quotes stripped.
//   '...'  — identical (no interpolation).
//   Empty quotes ("" or '') produce an empty-string token.
//   Unterminated quote → return -1.
// ---------------------------------------------------------------------------
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
		while (*read_ptr == ' ')
			read_ptr++;
		if (*read_ptr == '\0')
			break;

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
					*write_ptr++ = character;
				}
				break;

			case TOKENIZE_STATE_IN_SINGLE_QUOTE:
				if (character == '\'') {
					state = TOKENIZE_STATE_NORMAL;
				} else {
					*write_ptr++ = character;
				}
				break;
			}
		}

		if (state != TOKENIZE_STATE_NORMAL)
			return -1;

		*write_ptr++ = '\0';
		argv[argc++] = token_start;
	}

	return argc;
}

// ---------------------------------------------------------------------------
// Build "/bin/<cmd>" path into dst.
// ---------------------------------------------------------------------------
static void build_path(char *dst, size_t dst_size, const char *cmd)
{
	const char *prefix = "/bin/";
	size_t i = 0;
	while (*prefix && i < dst_size - 1)
		dst[i++] = *prefix++;
	while (*cmd && i < dst_size - 1)
		dst[i++] = *cmd++;
	dst[i] = '\0';
}

// ---------------------------------------------------------------------------
// Split a command line by unquoted '|' characters.
// Fills `stages[]` with pointers into `line` (which is modified in-place by
// replacing '|' with '\0').
// Returns the number of stages (>= 1), or 0 on error.
//
// Note: this is a simple scan — it does not handle '|' inside quotes.
// Shell pipelines with pipe characters in quoted arguments are unusual; for
// correctness, quote-aware splitting would require a full parser.
// ---------------------------------------------------------------------------
static int split_pipeline(char *line, char **stages, int max_stages)
{
	stages[0] = line;
	int count = 1;

	for (char *p = line; *p; p++) {
		if (*p == '|') {
			if (count >= max_stages)
				return 0;
			*p = '\0';
			stages[count++] = p + 1;
		}
	}
	return count;
}

// ---------------------------------------------------------------------------
// Execute a single pipeline stage.
//
// in_fd  — file descriptor to use as stdin  (0 = inherited; close after fork)
// out_fd — file descriptor to use as stdout (1 = inherited; close after fork)
//
// The stage string is tokenized and may contain a `< filename` redirect
// specifier which replaces stdin with the named file.
//
// Returns the child PID (>= 0) on success, -1 on error.
// ---------------------------------------------------------------------------
static int execute_stage(char *stage, int in_fd, int out_fd)
{
	const char *argv[MAX_ARGS + 1];
	int argc = tokenize(stage, argv, MAX_ARGS);
	if (argc <= 0)
		return -1;

	// Scan argv for a `<` redirect.  This modifies argc and argv[] in place
	// to remove the `<` and the filename token.
	int redirect_file_fd = -1;
	const char *redirect_argv[MAX_ARGS + 1];
	int redirect_argc = 0;

	for (int i = 0; i < argc; i++) {
		if (strcmp(argv[i], "<") == 0) {
			if (i + 1 >= argc) {
				print("shell: expected filename after '<'\r\n");
				return -1;
			}
			redirect_file_fd = open(argv[i + 1]);
			if (redirect_file_fd < 0) {
				print(argv[i + 1]);
				print(": no such file\r\n");
				return -1;
			}
			i++;  // skip filename
		} else {
			redirect_argv[redirect_argc++] = argv[i];
		}
	}
	redirect_argv[redirect_argc] = (const char *)0;

	if (redirect_argc == 0)
		return -1;

	char path[MAX_INPUT];
	build_path(path, MAX_INPUT, redirect_argv[0]);

	int pid = fork();
	if (pid < 0) {
		print("shell: fork failed\r\n");
		if (redirect_file_fd >= 0)
			close(redirect_file_fd);
		return -1;
	}

	if (pid == 0) {
		// Child: set up stdin/stdout redirects.
		if (in_fd != 0) {
			dup2(in_fd, 0);
			close(in_fd);
		}
		if (out_fd != 1) {
			dup2(out_fd, 1);
			close(out_fd);
		}
		if (redirect_file_fd >= 0) {
			dup2(redirect_file_fd, 0);
			close(redirect_file_fd);
		}
		execv(path, redirect_argv);
		// execv failed — print error and exit.
		print(redirect_argv[0]);
		print(": command not found\r\n");
		exit(127);
	}

	// Parent: close fds that were passed into the child.
	if (redirect_file_fd >= 0)
		close(redirect_file_fd);

	return pid;
}

// ---------------------------------------------------------------------------
// Execute a pipeline of one or more stages.
// For N stages, N-1 pipes are created between them.
// The parent waits for the last stage to exit.
// ---------------------------------------------------------------------------
static void execute_pipeline(char **stages, int n_stages)
{
	// For N stages we need N-1 pipes.
	// pipes[i][0] = read end of pipe between stage i and stage i+1.
	// pipes[i][1] = write end.
	int pipes[MAX_PIPELINE_STAGES - 1][2];

	for (int i = 0; i < n_stages - 1; i++) {
		if (pipe(pipes[i]) < 0) {
			print("shell: pipe creation failed\r\n");
			return;
		}
	}

	int pids[MAX_PIPELINE_STAGES];

	for (int i = 0; i < n_stages; i++) {
		int in_fd  = (i == 0)             ? 0 : pipes[i - 1][0];
		int out_fd = (i == n_stages - 1)  ? 1 : pipes[i][1];

		pids[i] = execute_stage(stages[i], in_fd, out_fd);

		// Parent closes the pipe ends it just handed to the child so EOF
		// propagates correctly once the writing child exits.
		if (i > 0)
			close(pipes[i - 1][0]);
		if (i < n_stages - 1)
			close(pipes[i][1]);
	}

	// Wait for all children.  Waiting in reverse order avoids the last stage
	// blocking on a full pipe from an earlier stage that we haven't waited for.
	// In practice, waiting only for the last stage is sufficient for most shell
	// pipelines, but waiting for all is correct.
	for (int i = n_stages - 1; i >= 0; i--) {
		if (pids[i] >= 0)
			wait(pids[i]);
	}
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

int main(void)
{
	char input[MAX_INPUT];
	char *stages[MAX_PIPELINE_STAGES];

	print("Bazzulto OS shell\r\n");

	for (;;) {
		print("bazzulto> ");
		size_t len = read_line(input, MAX_INPUT);

		if (len == 0)
			continue;

		// Split on '|' to detect pipelines.
		int n_stages = split_pipeline(input, stages, MAX_PIPELINE_STAGES);
		if (n_stages <= 0) {
			print("shell: too many pipeline stages\r\n");
			continue;
		}

		if (n_stages == 1) {
			// Single command: tokenize, check for builtins and redirects.
			const char *argv[MAX_ARGS + 1];
			int argc = tokenize(stages[0], argv, MAX_ARGS);
			if (argc < 0) {
				print("error: unterminated string\r\n");
				continue;
			}
			if (argc == 0)
				continue;

			// Check for `<` redirect without a pipeline.
			int has_redirect = 0;
			for (int i = 0; i < argc; i++) {
				if (strcmp(argv[i], "<") == 0) {
					has_redirect = 1;
					break;
				}
			}

			if (has_redirect) {
				// Use the pipeline execution path (handles `<` redirect).
				execute_pipeline(stages, 1);
			} else {
				// Plain spawn — faster path, no fork overhead.
				char path[MAX_INPUT];
				build_path(path, MAX_INPUT, argv[0]);
				argv[argc] = (const char *)0;
				int pid = spawn(path, argv);
				if (pid < 0) {
					print(argv[0]);
					print(": command not found\r\n");
					continue;
				}
				wait(pid);
			}
		} else {
			execute_pipeline(stages, n_stages);
		}
	}
}
