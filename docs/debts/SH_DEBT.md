# sh — Deuda técnica y features pendientes

Shell POSIX.1-2024 ubicada en `userspace/bin/sh/`.

Estado general: implementación completa del §2 Shell Command Language.
Las siguientes áreas están pendientes o parcialmente implementadas.

---

## 1. Sin impedimento externo — implementables en la shell

### 1.1 Builtin `test` / `[`

**Estado:** ❌ pendiente  
**Impedimento:** ninguno  
**Archivos a modificar:** `userspace/bin/sh/src/builtins.rs`

`test expr` y `[ expr ]` son builtins regulares (§2.15). Sin este builtin, los
scripts POSIX típicos (`if [ -f file ]`, `while [ $i -lt 10 ]`) requieren que
exista un binario externo en `/system/bin/[`.

Operadores a implementar:
- Archivos: `-e`, `-f`, `-d`, `-r`, `-w`, `-x`, `-s`, `-z`... (no disponibles sin `raw_fstat`)
- Strings: `-z str`, `-n str`, `str1 = str2`, `str1 != str2`
- Enteros: `-eq`, `-ne`, `-lt`, `-le`, `-gt`, `-ge`
- Booleanos: `!`, `-a`, `-o`, `( expr )`

Nota: `-e`, `-f`, `-d`, `-s` requieren `raw_fstat` (ya disponible en `bazzulto_system::raw`
desde 2026-04-14).

---

### 1.2 Builtin `printf`

**Estado:** ❌ pendiente  
**Impedimento:** ninguno  
**Archivos a modificar:** `userspace/bin/sh/src/builtins.rs`

`printf format [args...]` es §2.15. Formatos a soportar: `%s`, `%d`, `%i`, `%u`,
`%o`, `%x`, `%X`, `%f`, `%e`, `%g`, `%c`, `%%`, escapes `\n \t \r \\ \a \b`.

---

### 1.3 `(( expr ))` como comando standalone

**Estado:** ❌ pendiente  
**Impedimento:** ninguno  
**Archivos a modificar:** `userspace/bin/sh/src/parser.rs`, `executor.rs`

`$((...))` ya evalúa aritmética correctamente. `(( expr ))` como comando
independiente es idéntico en semántica pero retorna 0 si el resultado es distinto
de cero, 1 si es cero. Es una extensión bash/ksh ampliamente usada.

Implementación: detectar `((` como token en `parse_simple_or_compound` →
`TAG_ARITH_CMD`; en el executor evaluar con `arithmetic_expand` y retornar
`if result != 0 { 0 } else { 1 }`.

---

### 1.4 Variables locales en funciones (`local`)

**Estado:** ❌ pendiente  
**Impedimento:** diseño de `VarStore`  
**Archivos a modificar:** `userspace/bin/sh/src/vars.rs`, `builtins.rs`, `executor.rs`

POSIX §2.9.5 no requiere `local`, pero bash/ksh lo implementan. Sin `local`,
las funciones modifican las variables del entorno global.

Diseño propuesto: añadir un stack de frames a `VarStore`:
```rust
pub frames: Vec<Vec<(String, String)>>,  // (name, saved_value_or_sentinel)
```
`execute_function_call` hace push de un frame vacío al entrar y pop al salir,
restaurando las variables marcadas con `local`. `builtin_local` marca la variable
en el frame actual.

---

### 1.5 `set -e` (errexit) con semántica POSIX correcta

**Estado:** parcial  
**Impedimento:** ninguno  
**Archivos a modificar:** `userspace/bin/sh/src/executor.rs`

La implementación actual sale en cualquier status != 0. POSIX §2.8.1 especifica
que `-e` **no aplica** en:
- La condición de un `if`, `while`, `until` (el propio test)
- Comandos seguidos de `||` o `&&`
- Cualquier comando en una pipeline que no sea el último
- Comandos dentro de `!` negation
- Comandos dentro de un subshell usado como condición

Corrección: pasar un flag `in_condition: bool` por el árbol de ejecución para
suprimir errexit en los contextos correctos.

---

### 1.6 `trap` con handlers reales (ejecución de código shell)

**Estado:** parcial — solo ignore (`""`) y reset (`"-"`)  
**Impedimento:** diseño de señales async  
**Archivos a modificar:** `userspace/bin/sh/src/builtins.rs`, `main.rs`

POSIX §2.14: `trap 'cmd' SIGNAL` debe ejecutar `cmd` (código shell) cuando se
recibe la señal. Implementación actual solo llama `raw_sigaction` para ignorar
o restaurar señales.

Diseño propuesto:
1. Registrar un handler de señal en C/asm que setee un flag atómico por señal
   (`TRAP_PENDING[signal] = true`)
2. La shell chequea `TRAP_PENDING` entre comandos (en `execute_compound_list`)
3. Cuando hay señal pendiente: parsear y ejecutar el string de trap almacenado
   en `state.trap_table: Vec<(i32, String)>`

Requiere coordinación con la semántica de `async-signal-safety` del kernel.

---

## 2. Dependen de funcionalidad del VFS/kernel

### 2.1 Pathname expansion / globbing (`*`, `?`, `[...]`)

**Estado:** ❌ pendiente  
**Impedimento:** integración con `raw_getdents64`  
**Archivos a modificar:** `userspace/bin/sh/src/expand.rs`

`pattern_matches(pattern, subject)` ya está implementada en `expand.rs` (§2.13).
Falta la función que la aplica sobre las entradas de un directorio:

```rust
fn glob_expand(pattern: &str) -> Vec<String> {
    // 1. Separar directorio y parte de patrón: "src/*.rs" → dir="src/", pat="*.rs"
    // 2. raw_getdents64(dir_fd, buf, buf_len) para listar entradas
    // 3. Para cada entrada: if pattern_matches(pat, entry_name) → incluir
    // 4. Si no hay matches: retornar el patrón literal (comportamiento POSIX)
}
```

`raw_getdents64` ya existe en `bazzulto_system::raw`. El formato de las entradas
debe verificarse contra `kernel/src/syscall/mod.rs` (struct `linux_dirent64`).

---

### 2.2 Job control (`fg`, `bg`, `jobs`)

**Estado:** ❌ pendiente  
**Impedimento:** el kernel no expone control completo de grupos de proceso  
**Archivos a modificar:** kernel + `userspace/bin/sh/src/`

Job control requiere:
- `setpgrp` / `getpgrp` — no expuestos en vDSO
- Señales `SIGTSTP`, `SIGCONT` con semántica de terminal
- `tcsetpgrp` — control del grupo de foreground del terminal

`raw_setfgpid` existe pero es insuficiente. Necesita trabajo en el kernel
(`kernel/src/syscall/mod.rs`) para exponer `SLOT_SETPGRP` y manejar
`SIGTSTP`/`SIGCONT` correctamente en el scheduler.

---

## 3. Sin dependencia del kernel, pero alto esfuerzo

### 3.1 Historial de comandos y edición de línea

**Estado:** ❌ pendiente  
**Impedimento:** no existe librería de línea de comandos en `no_std`  
**Archivos a modificar:** `userspace/bin/sh/src/main.rs` (REPL loop)

Requiere implementar desde cero:
- Buffer de edición con cursor móvil
- ANSI escape sequences para mover el cursor (`\x1b[D`, `\x1b[C`, etc.)
- Historial en memoria (`Vec<String>`) con navegación por flechas
- Autocompletado básico de comandos (opcional, requiere globbing)

La shell actualmente lee línea a línea con `raw_read` carácter a carácter sin
modo raw del terminal.

---

## 4. Extensiones bash — fuera de POSIX.1-2024 §2

Estos items **no son parte del spec objetivo** (POSIX.1-2024 §2 Shell Command
Language). Se documentan para referencia futura.

| Feature | Nota |
|---|---|
| `[[ ]]` conditional command | Extensión bash/ksh. Semántica diferente a `[`: no hace word splitting, pattern matching distinto. |
| `select` | Extensión KornShell. Genera menús interactivos. |
| Arrays `arr=(a b c)` | Extensión bash. POSIX no tiene arrays. |
| Process substitution `<(cmd)` | Extensión bash. Requiere named pipes o `/proc/fd`. |
| `{a..z}` brace expansion | Extensión bash. No es parte de POSIX. |

---

## Referencias

- POSIX.1-2024 §2: `docs/posix-spec/`
- Implementación: `userspace/bin/sh/src/`
- Tests: `userspace/bin/sh-tests/tests/`
- Plan original: `~/.claude/plans/glistening-dazzling-parnas.md`
