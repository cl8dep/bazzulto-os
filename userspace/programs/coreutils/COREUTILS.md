# Coreutils Implementation Tracker

POSIX.1-2024 utility list. Source: [pubs.opengroup.org/onlinepubs/9799919799/](https://pubs.opengroup.org/onlinepubs/9799919799/)

Status legend:
- `[x]` — implemented
- `[ ]` — not implemented
- `[~]` — partial / non-POSIX (Bazzulto-specific variant)
- `[–]` — not applicable (shell built-in, SCCS legacy, or out of scope for v1.0)

---

## Implemented

| Utility | Notes |
|---------|-------|
| `basename` | |
| `cat` | `-u` accepted (no-op); `-` as stdin; continues on error; POSIX exit codes |
| `cksum` | ISO/IEC 8802-3 CRC-32; POSIX normative algorithm |
| `df` | `-k`, `-P`, `-t` (XSI); 512/1024-byte units; uses kernel `GETMOUNTS` syscall |
| `diff` | `-b`, `-c`/`-C n`, `-e`, `-f`, `-u`/`-U n`, `-r`; Myers O(ND) |
| `ls` | `-a`, `-A`, `-d`, `-F`, `-i`, `-l`, `-n`, `-p`, `-q`, `-R`, `-r`, `-S`, `-t`, `-1`; one-per-line output; timestamps/owner/group are TODO (kernel fstat does not yet expose them) |
| `mkdir` | `-p` (create parents, POSIX intermediate-mode formula); `-m mode` (octal and symbolic: `u=rwx`, `a+w`, `o-x`, etc.); proper errno messages |
| `printf` | `%d` `%i` `%u` `%o` `%x` `%X` `%s` `%b` `%c` `%%`; flags `-` `0` `+` ` `; width; precision; `%n$` numbered args; format reuse; `\c` stop; octal `\ddd`/`\0ddd`; overflow diagnostics |
| `false` | |
| `true` | |

Non-POSIX Bazzulto utilities (same binary):

| Utility | Notes |
|---------|-------|
| `reboot` | |
| `shutdown` | |

### Shell

| Utility | Status | Notes |
|---------|--------|-------|
| `sh` | `[~]` | Spec 2.2.1–2.2.4 implemented (quoting: backslash, single-quotes, double-quotes, dollar-single-quotes); simple commands, pipelines, redirects (`<`, `>`, `>>`), builtins: `cd`, `exit`, `pwd`, `echo`, `export`, `shift`, `jobs` |

---

## Needs Review — previously written, pending POSIX pass

These utilities have existing implementations but have not yet been reviewed against POSIX.1-2024.

| Utility | Priority | Notes |
|---------|----------|-------|
| `cp` | high | |
| `cut` | medium | |
| `date` | medium | |
| `dirname` | low | |
| `echo` | medium | |
| `env` | medium | |
| `grep` | high | |
| `head` | medium | |
| `kill` | medium | |
| `ls` | high | done — moved to Implemented |
| `mkdir` | medium | done — moved to Implemented |
| `mv` | high | |
| `printf` | medium | done — moved to Implemented |
| `ps` | low | Bazzulto-specific; not POSIX |
| `pwd` | low | shell built-in; standalone needs review |
| `rm` | high | |
| `sleep` | low | |
| `sort` | medium | |
| `tail` | medium | |
| `tee` | medium | |
| `time` | low | |
| `touch` | medium | |
| `tr` | medium | |
| `uniq` | medium | |
| `wc` | medium | |
| `yes` | low | |

---

## Not Implemented — POSIX utilities

| Utility | Priority | Notes |
|---------|----------|-------|
| `ar` | low | object archiver |
| `asa` | low | carriage-control filter |
| `at` | low | job scheduling |
| `awk` | high | |
| `batch` | low | |
| `bc` | medium | |
| `cal` | medium | |
| `chgrp` | medium | requires groups |
| `chmod` | high | requires permissions model |
| `chown` | high | requires permissions model |
| `cmp` | medium | |
| `comm` | low | |
| `compress` | low | |
| `crontab` | low | |
| `csplit` | low | |
| `ctags` | low | |
| `cxref` | low | |
| `dd` | medium | |
| `du` | medium | requires `fs_stats` per-directory |
| `ed` | low | line editor |
| `ex` | low | |
| `expand` | low | tab→space |
| `expr` | medium | |
| `fc` | low | shell history; shell built-in in POSIX |
| `file` | medium | |
| `find` | high | |
| `fold` | low | |
| `fuser` | low | |
| `getconf` | low | |
| `getopts` | low | shell built-in |
| `id` | medium | requires user/group model |
| `iconv` | low | |
| `ipcrm` | low | IPC |
| `ipcs` | low | IPC |
| `join` | low | |
| `lex` | low | |
| `ln` | high | |
| `locale` | low | |
| `localedef` | low | |
| `logger` | low | |
| `logname` | low | |
| `lp` | low | printing |
| `m4` | low | |
| `mailx` | low | |
| `make` | low | |
| `man` | medium | |
| `mesg` | low | |
| `mkfifo` | medium | named pipes |
| `more` | medium | pager |
| `msgfmt` | low | i18n |
| `newgrp` | low | |
| `ngettext` | low | i18n |
| `nice` | medium | |
| `nl` | low | number lines |
| `nm` | low | symbol table |
| `nohup` | medium | |
| `od` | medium | octal dump |
| `paste` | low | |
| `patch` | medium | |
| `pathchk` | low | |
| `pax` | low | archive |
| `pr` | low | |
| `read` | low | shell built-in |
| `readlink` | medium | |
| `realpath` | medium | |
| `renice` | low | |
| `rmdir` | medium | |
| `sed` | high | |
| `split` | low | |
| `strings` | low | |
| `strip` | low | |
| `stty` | low | terminal settings |
| `tabs` | low | |
| `talk` | low | |
| `test` | high | |
| `timeout` | medium | |
| `tsort` | low | |
| `tty` | low | |
| `uname` | medium | |
| `uncompress` | low | |
| `unexpand` | low | |
| `unlink` | medium | |
| `uucp` | low | |
| `uudecode` | low | |
| `uuencode` | low | |
| `uustat` | low | |
| `uux` | low | |
| `vi` | low | |
| `wait` | medium | shell built-in / standalone |
| `who` | low | |
| `write` | low | |
| `xargs` | high | |
| `xgettext` | low | i18n |
| `yacc` | low | |
| `zcat` | low | |

---

## Not Applicable

Shell built-ins with no standalone POSIX binary form, SCCS legacy tools, or out-of-scope for v1.0:

`alias`, `bg`, `break`, `cd`, `colon`, `command`, `continue`, `delta`, `dot`,
`eval`, `exec`, `exit`, `export`, `fg`, `get`, `hash`, `jobs`, `prs`, `readonly`,
`return`, `rmdel`, `sact`, `sccs`, `set`, `sh`, `shift`, `times`, `trap`, `type`,
`ulimit`, `umask`, `unalias`, `unget`, `unset`, `val`, `what`
