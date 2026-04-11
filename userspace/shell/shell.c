#include "../library/systemcall.h"
#include "string.h"
// creat() declared in systemcall.h (syscall 24)

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
// The kernel TTY layer handles echo, backspace, and line editing — the shell
// receives a complete line after the user presses Enter.
static size_t read_line(char *buf, size_t max)
{
	int64_t n = read(0, buf, max - 1);
	if (n <= 0)
		return 0;
	// Strip trailing newline delivered by the TTY.
	if (n > 0 && buf[n - 1] == '\n')
		n--;
	buf[n] = '\0';
	return (size_t)n;
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

	// Scan argv for `<`, `>`, and `>>` redirects.
	// Removes these tokens (and their filename arguments) from the command argv.
	int redirect_in_fd  = -1;   // stdin  redirect (<)
	int redirect_out_fd = -1;   // stdout redirect (> or >>)
	const char *redirect_argv[MAX_ARGS + 1];
	int redirect_argc = 0;

	for (int i = 0; i < argc; i++) {
		if (strcmp(argv[i], "<") == 0) {
			if (i + 1 >= argc) {
				print("shell: expected filename after '<'\r\n");
				return -1;
			}
			redirect_in_fd = open(argv[i + 1]);
			if (redirect_in_fd < 0) {
				print(argv[i + 1]);
				print(": no such file\r\n");
				return -1;
			}
			i++;  // skip filename
		} else if (strcmp(argv[i], ">>") == 0) {
			if (i + 1 >= argc) {
				print("shell: expected filename after '>>'\r\n");
				return -1;
			}
			// Create-or-open, then seek to end.
			redirect_out_fd = creat(argv[i + 1]);
			if (redirect_out_fd < 0) {
				print(argv[i + 1]);
				print(": cannot open\r\n");
				return -1;
			}
			seek(redirect_out_fd, 0, 2);  // SEEK_END = 2
			i++;
		} else if (strcmp(argv[i], ">") == 0) {
			if (i + 1 >= argc) {
				print("shell: expected filename after '>'\r\n");
				return -1;
			}
			redirect_out_fd = creat(argv[i + 1]);
			if (redirect_out_fd < 0) {
				print(argv[i + 1]);
				print(": cannot create\r\n");
				return -1;
			}
			i++;
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
		if (redirect_in_fd >= 0)  close(redirect_in_fd);
		if (redirect_out_fd >= 0) close(redirect_out_fd);
		return -1;
	}

	if (pid == 0) {
		// Child: set up stdin/stdout redirects.
		// Pipeline fds first, then explicit redirects (redirects win).
		if (in_fd != 0) {
			dup2(in_fd, 0);
			close(in_fd);
		}
		if (out_fd != 1) {
			dup2(out_fd, 1);
			close(out_fd);
		}
		if (redirect_in_fd >= 0) {
			dup2(redirect_in_fd, 0);
			close(redirect_in_fd);
		}
		if (redirect_out_fd >= 0) {
			dup2(redirect_out_fd, 1);
			close(redirect_out_fd);
		}
		// Close all inherited fds >= 3. The child inherits the parent's full fd
		// table, including both ends of every pipeline pipe. If we leave the
		// write end open, pipe_write_end_open() will find it in our own table
		// and pipe_read() will never see EOF — causing all readers to hang.
		for (int close_fd = 3; close_fd < 64; close_fd++)
			close(close_fd);
		execv(path, redirect_argv);
		// execv failed — print error and exit.
		print(redirect_argv[0]);
		print(": command not found\r\n");
		exit(127);
	}

	// Parent: close fds that were handed to the child.
	if (redirect_in_fd >= 0)  close(redirect_in_fd);
	if (redirect_out_fd >= 0) close(redirect_out_fd);

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

	// Declare the last stage as the terminal foreground process so that
	// Ctrl+C (SIGINT) is delivered to it while the shell is blocked in wait().
	if (pids[n_stages - 1] >= 0)
		set_terminal_foreground_pid(pids[n_stages - 1]);

	// Wait for all children.  Waiting in reverse order avoids the last stage
	// blocking on a full pipe from an earlier stage that we haven't waited for.
	// In practice, waiting only for the last stage is sufficient for most shell
	// pipelines, but waiting for all is correct.
	for (int i = n_stages - 1; i >= 0; i--) {
		if (pids[i] >= 0)
			wait(pids[i]);
	}

	// Shell regains the terminal — Ctrl+C must no longer kill anything.
	set_terminal_foreground_pid(0);
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
			// Scan the raw string for redirect operators before tokenizing,
			// because tokenize() modifies the buffer in place (inserts \0).
			// A second tokenize in execute_stage would see a truncated string.
			int has_redirect = 0;
			for (const char *scan = stages[0]; *scan; scan++) {
				if (*scan == '<' || *scan == '>') {
					has_redirect = 1;
					break;
				}
			}

			if (has_redirect) {
				// Use the pipeline execution path (handles redirects).
				execute_pipeline(stages, 1);
			} else {
				// Single command without redirects: tokenize and spawn.
				const char *argv[MAX_ARGS + 1];
				int argc = tokenize(stages[0], argv, MAX_ARGS);
				if (argc < 0) {
					print("error: unterminated string\r\n");
					continue;
				}
				if (argc == 0)
					continue;
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
				set_terminal_foreground_pid(pid);
				wait(pid);
				set_terminal_foreground_pid(0);
			}
		} else {
			execute_pipeline(stages, n_stages);
		}
	}
}
