# Bazzulto OS — Deuda Técnica

Documento de deuda técnica activa. Documenta únicamente lo pendiente de implementar.
Lo ya implementado no se registra aquí.

**Actualizado:** 2026-04-13 (post-v1.0)

---

## Bloque 1 — Syscalls y comportamiento parcialmente implementado

### 1a. `chmod` / `chown` / `fchmod` / `fchown` — stubs

Retornan 0 sin hacer nada. Requieren almacenamiento de `owner_uid`/`owner_gid` por
inode. FAT32 no tiene soporte nativo de permisos POSIX en disco. BAFS puede
implementarlo como extended attribute.

**Prerequisito:** BAFS como filesystem oficial (post-v1.0), o xattrs en FAT32.

### 1b. `CLOCK_REALTIME` retorna tiempo monotónico

`clock_gettime(CLOCK_REALTIME)` devuelve `CNTPCT_EL0`-derived ticks en lugar del
tiempo de pared real. El PL031 RTC está inicializado pero su valor no se usa como
base para `CLOCK_REALTIME`.

**Fix:** Leer el RTC en boot (`pl031_read()`), almacenar `wall_clock_base_seconds`,
y sumar el offset monotónico en `sys_clock_gettime` para `CLOCK_ID == 0`.

**Archivo:** `syscall/mod.rs` (clock_gettime), `hal/pl031_rtc.rs`.

### 1c. `MAP_SHARED` file-backed — stub

`sys_mmap` con `MAP_SHARED | fd` cae en el path de `MAP_PRIVATE`. Dos procesos
que mapeen el mismo inode con `MAP_SHARED` no verán escrituras del otro.

**Fix:** Implementar `FileBackedSharedPageRegistry` keyed por `(inode_id, page_index)`.
En fork, páginas `MAP_SHARED` no se marcan CoW — se reutiliza la misma página física.

**Archivos:** `process/mod.rs` (MmapBacking::SharedFile variant), `memory/mod.rs`
(page fault handler), `scheduler/mod.rs` (fork).

### 1d. FD table compartida en threads

`clone(CLONE_VM | CLONE_THREAD)` copia la FD table en lugar de compartirla. Un
`close(fd)` en un thread no afecta a los demás threads del mismo proceso.

**Fix:** Envolver `FdTable` en `Arc<SpinLock<FdTable>>`. `fork()` clona el Arc
(copia independiente). `clone_thread()` clona el puntero (tabla compartida).

**Archivos:** `process/mod.rs`, `scheduler/mod.rs`, `syscall/mod.rs`.

### 1e. `sendmsg` / `recvmsg` SCM_RIGHTS

`sendmsg`/`recvmsg` manejan datos normales pero no `SCM_RIGHTS` (transferencia de
file descriptors via control message). Requerido por D-Bus, Wayland, ssh-agent.

**Fix:** Parsear `cmsghdr` con `cmsg_type == SCM_RIGHTS`; duplicar cada fd en la
FD table del receptor. Validar contra `granted_permissions` del receptor
(requiere Binary Permission Model Tier 2/3).

**Archivos:** `syscall/mod.rs` (sys_sendmsg, sys_recvmsg).

---

## Bloque 2 — Memoria

### 2a. VMA sigue siendo `Vec`

`mmap_regions: Vec<MmapRegion>` tiene un cap implícito y búsqueda O(n). Bajo carga
con muchos `mmap` / `munmap` esto degrada el rendimiento.

**Fix:** Reemplazar con `BTreeMap<u64, MmapRegion>` keyed por `start_address`.
Eliminar el límite de 1024 regiones.

**Archivo:** `process/mod.rs`.

### 2b. Buddy allocator

El allocator físico usa first-fit O(n). Bajo carga real con allocaciones/
liberaciones frecuentes, la fragmentación crece.

**Fix:** Buddy allocator binario (12 órdenes, 4 KB–16 MB). Mantener first-fit
como fallback para rangos no-alineados.

**Archivo:** `memory/physical.rs`.

### 2c. ASLR entropy pool

`CNTPCT_EL0` como única fuente de entropía para ASLR es predecible bajo ciertas
condiciones (boot determinístico en VM).

**Fix:** Pool mezclado de `CNTPCT_EL0 ^ CNTFRQ_EL0 ^ MIDR_EL1 ^ PA_stack_inicial`.
LFSR Galois de 64 bits tras cada extracción.

**Archivos:** Nuevo `memory/entropy.rs`; usar en `loader/mod.rs`, `process/mod.rs`.

---

## Bloque 3 — Filesystem

### 3d. BAFS: truncate con extent freeing

`BafsFileInode::truncate()` retorna `NotSupported`. Truncar un archivo no libera
los extents que ya no son necesarios.

**Archivo:** `fs/bafs_driver.rs`, y el módulo `bafs` del submodule.

### 3e. BAFS como filesystem por defecto

FAT32 sigue siendo el filesystem del sistema. BAFS está soportado como driver
opcional pero no es el default. Cambiar esto requiere:
- Herramienta `bafs-mkfs` en userspace para crear imágenes BAFS.
- Decisión de migración del rootfs.

**Prerequisito:** userspace tooling, no kernel.

---

## Bloque 4 — Seguridad y permisos (post-v1.0)

Estos ítems requieren `permissiond` en userspace o componentes aún no disponibles.
El lado kernel del Binary Permission Model (Tier 1/4, `granted_permissions`,
`vfs_open` check, `sys_mount` check) está implementado en v1.0.

### 4a. Tier 2 — policy store

`//sys:policy:{sha256}` en `vfs_open()`: verifica política firmada almacenada
por `permissiond`. Requiere daemon corriendo y protocolo IPC definido.

### 4b. Tier 3 — ELF section + approval prompt

Binarios con `.bazzulto_permissions` ELF section que declaran sus permisos
explícitamente. Requiere `permissiond` + UI (terminal o gráfica) para el prompt
de aprobación del usuario.

### 4c. Ed25519 Tier 1 signature verification

Verificar `.baz_sig` ELF note contra clave pública embedded en kernel. Actualmente
`//system:/bin/**` recibe full trust por path sin verificación criptográfica.

**Prerequisito:** herramienta de signing en toolchain Bazzulto.

### 4d. `sys_request_cap` — runtime elevation

Syscall para que un proceso pida permisos adicionales en tiempo de ejecución.
Con rate limiting de 3 denials/60s. Requiere `permissiond`.

### 4e. `sys_powerbox_open` — file picker sin exponer path

File picker que retorna un fd al proceso sin revelar el path. Requiere `permissiond`
+ UI gráfica o terminal.

### 4f. `sys_restrict_self` — reducción irreversible de privilegios

Para interpreters (Python, Lua, etc.) que quieren ejecutar código no confiable.
Requiere Tier 2/3 completos.

### 4g. Password re-prompt para acciones Authenticated

`//sys:mount/**`, `//sys:driver/**`, etc. actualmente solo verifican que el
proceso tenga el `ActionPermission` en `granted_actions`. El re-prompt de
contraseña real requiere `permissiond` + credenciales.

### 4h. IPC fd re-validation en SCM_RIGHTS

Al recibir un fd via `recvmsg(SCM_RIGHTS)`, verificar que el receptor tiene los
permisos de acceso correspondientes al inode subyacente. Requiere Binary
Permission Model Tier 2/3.

### 4i. Saved-set-UID

`setuid()` con semántica POSIX completa requiere tres UIDs: real, effective,
saved. Actualmente solo se manejan real y effective.

### 4j. `seccomp` filter

Filtrado de syscalls por proceso. Deferred hasta que el modelo de seguridad
esté estable con Tier 2/3.

### 4k. TPM binding para policy entries

Policy entries firmadas y enlazadas a un TPM específico. Deferred hasta
hardware real (no QEMU).

### 4l. Merkle root de dependencias dinámicas

Para Tier 1 con dynamic linking: computar Merkle root incluyendo todas las `.so`
dependencias. Requiere dynamic linker.

---

## Bloque 5 — IPC avanzado (post-v1.0)

### 5a. `FUTEX_REQUEUE` / `FUTEX_WAIT_BITSET`

Operaciones futex avanzadas para compatibilidad con glibc condvars. `FUTEX_WAIT`
y `FUTEX_WAKE` básicos están implementados.

### 5b. `CLONE_SIGHAND` / `CLONE_FS` / `CLONE_NEWNS`

Flags de clone avanzados. `CLONE_VM | CLONE_THREAD` está implementado.

### 5c. `splice()` / `vmsplice()`

Zero-copy transfer entre pipes y fds. No implementado.

---

## Bloque 6 — Scheduling (post-v1.0)

### 6a. `Arc<PageTable>` en clone de threads

`clone()` de threads usa raw pointer para compartir el page table en lugar de
`Arc<PageTable>`. Documentado como TECHNICAL DEBT en el código.

### 6b. Orphan reparenting completo

Cuando un proceso padre muere, sus hijos huérfanos deben ser reparentados a
PID 1 (bzinit). El mecanismo existe pero no está garantizado para todos los
escenarios de exit.

---

## Bloque 7 — Red (explícitamente fuera de scope)

No hay networking. Deferred indefinidamente.

- VirtIO-net driver (device ID=1, features `VIRTIO_NET_F_MAC`)
- Ethernet + ARP cache
- IPv4 + ICMP + UDP + TCP
- BSD socket API (`AF_INET`)
- DHCP client
- DNS stub resolver

Alternativa: integrar `smoltcp` (no_std Rust TCP/IP stack) cuando se retome.

---

## Bloque 8 — Userspace / BDL (no es kernel)

Estos ítems son trabajo de userspace, no de kernel. Se documentan aquí porque
bloquean funcionalidad end-to-end.

### 8a. `permissiond` daemon

Primer binario Tier 1 del sistema. Gestiona el policy store (Tier 2), los prompts
de aprobación (Tier 3), y los re-prompts de contraseña (Authenticated).

**Prerequisito:** scheduler + IPC estables en kernel (✅ ya disponibles).

### 8b. Dynamic linker / PT_INTERP

`sys_exec` detecta `PT_INTERP` pero no lo ejecuta. El dynamic linker es userspace.

**Prerequisito:** `mmap` file-backed MAP_PRIVATE (✅ implementado en v1.0).

### 8c. `baz` package manager — `grant_permissions()`

El package manager necesita un mecanismo para otorgar permisos a los paquetes
instalados. Requiere `permissiond`.

### 8d. Text editor (nano-like)

No existe editor de texto en userspace. Todos los prerequisitos de kernel
(PTY raw mode, `TIOCGWINSZ`, VFS read/write) están disponibles.

### 8e. Bazzulto Standard Library (BSL)

API stable para userspace: I/O, memory, permissions, IPC, crypto. Sin versión
estable publicada.

### 8f. `bzctl` timeline / dependency graph

`bzctl start/stop` básico funciona. El timeline de arranque y el grafo de
dependencias de servicios no está implementado en bzinit.

---

## Bloque 9 — Console / Teclado

### 9a. `console_putc()` no thread-safe

El buffer UTF-8 interno de `console_putc()` no tiene spinlock. Escrituras
concurrentes desde múltiples cores pueden corromper la secuencia UTF-8.

**Archivo:** `drivers/console/console.c`.

### 9b. Key repeat rate no configurable

La tasa de repetición de teclas no es configurable desde el guest.

**Archivo:** `platform/qemu_virt/keyboard_virtio.c`.

---

## Notas de diseño activas

### BAFS vs FAT32

FAT32 es el filesystem del sistema para v1.0. BAFS está soportado como driver
opcional. El cambio a BAFS como default requiere `bafs-mkfs` en userspace y
una decisión de migración explícita — no es deuda de kernel.

### Binary Permission Model — estado en v1.0

Implementado en kernel: Tier 1 (system binary path → full trust), Tier 4
(sin `.bazzulto_permissions` section → hereda del padre + warning), checks en
`vfs_open()`, checks en `sys_mount()`/`sys_umount()`, `INODE_KERNEL_EXEC_ONLY`
para bzinit, `granted_permissions`/`granted_actions` en Process, herencia en
fork(), limpieza en exec() con re-asignación por tier.

Deferred: Tier 2 (policy store), Tier 3 (ELF section + prompt), Ed25519, SCM_RIGHTS
re-validation, password re-prompt, `sys_request_cap`, `sys_powerbox_open`,
`sys_restrict_self`.

### Red

TCP/IP está explícitamente fuera del scope de v1.0 y post-v1.0 sin fecha definida.
No es deuda — es una decisión de prioridad documentada.
