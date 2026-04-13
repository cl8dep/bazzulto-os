# Bazzulto OS — Deuda Técnica Unificada

Documento consolidado. Reemplaza los archivos individuales en `docs/debts/` y
`docs/tech-debt/`. Cada sección indica severidad, área afectada y prerequisitos.

**Actualizado:** 2026-04-13

---

## Estado de implementación — resumen

| Subsistema | Estado |
|---|---|
| Boot / Excepciones / EL0 trampoline | ✅ Completo |
| Memoria física + virtual (4-level pgtable) | ✅ Funcional |
| Heap kernel (slab + first-fit) | ✅ Funcional |
| HAL: DTB, Platform trait, GICv2, PL031 RTC | ✅ Funcional |
| Scheduler SMP per-CPU + work stealing + nice/priorities | ✅ Funcional |
| SMP: AP boot, per-CPU GIC + timer | ✅ Funcional (70%) |
| Proceso: fork CoW + exec + clone threads | ✅ Funcional |
| ELF loader ET_EXEC + ET_DYN/PIE | ✅ Funcional |
| Guard pages bajo kernel stack | ✅ Completo |
| Señales + sigprocmask/sigpending/sigsuspend + SIGSTOP/CONT | ✅ Funcional |
| Process groups + sessions + rlimits | ✅ Funcional |
| Syscalls (~100 definidas, ~75 implementadas) | ⚠️ Parcial |
| VFS + tmpfs + devfs + procfs + mount table | ✅ Funcional |
| /proc/self symlink + resolución de symlinks en VFS | ✅ Completo |
| SIGALRM / alarm() | ✅ Completo |
| TLB shootdown (tlbi vmalle1is) | ✅ Completo |
| IPC: pipes, FIFO, Unix sockets, semáforos, mqueues, futex | ✅ Funcional |
| IPC discriminante nlinks (sem/socket/mqueue) | ✅ Completo |
| epoll + poll + select | ✅ Funcional (select pendiente) |
| PTY + ioctl(TIOCGWINSZ/TIOCSWINSZ) | ✅ Funcional |
| Terminal signals (Ctrl+C/Z → SIGINT/SIGTSTP) | ✅ Completo |
| POSIX threads (clone + TLS + shared FD) | ⚠️ FD table no compartida |
| CLOCK_REALTIME + CLOCK_MONOTONIC | ✅ Funcional |
| CoW fork | ✅ Funcional |
| umask | ✅ Completo |
| envp en exec | ✅ Completo |
| sigaltstack + SA_ONSTACK | ✅ Completo |
| vDSO clock_gettime acelerado | ✅ Completo |
| Reboot / shutdown via PSCI | ✅ Completo |
| bzinit (PID 1, service manager) | ✅ Funcional |
| FAT32 | ⚠️ Parcial — sin instancia por volumen |
| BlockDevice trait + multi-disk registry | ✅ Implementado (actualizar tabla de estado) |
| Partition table (MBR/GPT) | ✅ Implementado (actualizar tabla de estado) |
| Demand paging (lazy BSS) | ❌ No implementado |
| MAP_SHARED + mmap file-backed | ❌ No implementado |
| MAP_SHARED excluido de CoW en fork() | ❌ No implementado |
| UID/GID + permisos de archivo POSIX | ❌ No implementado |
| Binary Permission Model (kernel side) | ❌ No implementado |
| Red (TCP/IP) | ❌ No implementado — explícitamente fuera de scope v1.0 |
| Binary Permission Model (permissiond daemon) | ❌ Post-v1.0 |
| Editor de texto | ❌ Post-v1.0 (userspace) |

---

## Bloque 1 — FAT32 (P1)

**Severidad:** Alta — sin esto no hay acceso a disco real.

### Estado actual

`Fat32DirInode` y `Fat32FileInode` implementan el trait `Inode` y funcionan.
Problemas pendientes:

- `fat32.rs` usa `static STATE: SyncCell<Option<Fat32State>>` — una única instancia
  global. No soporta múltiples volúmenes ni particiones.
- No existe un trait `BlockDevice` — el código llama funciones estáticas de virtio_blk.
- No hay registro global de discos.
- No hay parser de tabla de particiones (MBR/GPT).
- FAT32 no se auto-monta vía la tabla VFS.

### Paso 1 — Trait BlockDevice (`kernel/src/hal/disk.rs`)

```rust
pub trait BlockDevice: Send + Sync {
    fn read_sectors(&self, lba: u64, count: u32, buf: &mut [u8]) -> bool;
    fn write_sectors(&self, lba: u64, count: u32, buf: &[u8]) -> bool;
    fn sector_count(&self) -> u64;
    fn sector_size(&self) -> u32 { 512 }
    fn name(&self) -> &str;
}

static DISK_REGISTRY: SpinLock<Vec<Arc<dyn BlockDevice>>>;
pub fn register_disk(dev: Arc<dyn BlockDevice>);
pub fn disk_count() -> usize;
pub fn get_disk(index: usize) -> Option<Arc<dyn BlockDevice>>;
```

Envolver `virtio_blk` en `VirtioBlkDevice: BlockDevice`. `platform_init()` llama
`register_disk(...)` tras la enumeración virtio.

### Paso 2 — Parser de particiones (`kernel/src/fs/partition.rs`, nuevo)

MBR: leer sector 0, verificar `[510..512] == [0x55, 0xAA]`, parsear 4 entradas
en offsets 446/462/478/494 (byte 4 = tipo, bytes 8..12 = LBA start, 12..16 = count).

GPT: entrada MBR tipo 0xEE → leer LBA 1, verificar `b"EFI PART"`, parsear GUID
partition entries. FAT32 GUID = `{28732AC1-1FF8-D211-BA4B-00A0C93EC93B}`.

Sin MBR válido: un `Partition` cubriendo disco completo (start_lba=0).

### Paso 3 — Fat32Volume por instancia (`kernel/src/fs/fat32.rs`)

Eliminar `static STATE`. Añadir:

```rust
pub struct Fat32Volume {
    partition: Partition,
    bpb_bytes_per_sector: u32,
    bpb_sectors_per_cluster: u32,
    fat_start_lba: u64,
    data_start_lba: u64,
    root_cluster: u32,
    inner: SpinLock<Fat32VolumeInner>,
}
```

`Fat32DirInode` y `Fat32FileInode` pasan a mantener `Arc<Fat32Volume>`.

### Paso 4 — Auto-mount

En `kernel/src/fs/mod.rs`, `disk_init_and_mount()`:

```
for each disk → enumerate_partitions() → Fat32Volume::probe() → vfs_mount("/mnt/disk0p0", root)
```

### Paso 5 — Eliminar bypass path

- Eliminar rama `if path.starts_with("//disk:")` en `sys_open()`.
- Eliminar variante `FileDescriptor::FatFile`.
- Eliminar `strip_disk_prefix()`.

**Archivos:** `hal/disk.rs`, `platform/qemu_virt/virtio_blk.rs`, `fs/partition.rs`
(nuevo), `fs/fat32.rs`, `fs/vfs.rs`, `syscall/mod.rs`, `fs/mod.rs`.

---

## Bloque 2 — Memoria

**Severidad:** Media-Alta.

### 2a. Demand paging — lazy BSS

Segmentos ELF donde `p_filesz < p_memsz`: mapear BSS como not-present (PTE
present=0). En page fault (EC 0x24), si VA está en zona LazyZero → alojar página
zerada, mapear RW.

Requiere `vm_areas: Vec<VmArea>` en `Process` con `(start_va, end_va, VmAreaKind)`.

**Archivos:** `loader/mod.rs`, `process/mod.rs`, `memory/virtual_memory.rs`.

### 2b. MAP_SHARED excluido de CoW en fork()

`cow_copy_user()` marca todas las páginas RW como CoW, incluyendo páginas de
`MAP_SHARED`. Padre e hijo deben ver las mismas páginas físicas sin CoW.

Fix: tras `cow_copy_user()`, recorrer la `SharedRegionTable` y restaurar el
mapeo RW original en el hijo para cada región compartida.

**Archivos:** `scheduler/mod.rs` (fork), `memory/virtual_memory.rs`.

### 2c. Buddy allocator

El allocator físico es una free-list (O(n)). Implementar buddy allocator binario
(12 órdenes, 4 KB–16 MB) para evitar fragmentación bajo carga real.

**Archivo:** `memory/physical.rs`.

### 2d. ASLR entropy pool

`CNTPCT_EL0` como única fuente de entropía es predecible. Añadir
`memory/entropy.rs` con pool mezclado de `CNTPCT_EL0 + CNTFRQ_EL0 + MIDR_EL1 +
PA_stack_inicial`. LFSR Galois de 64 bits tras cada extracción.

**Archivos:** Nuevo `memory/entropy.rs`, `loader/mod.rs`, `process/mod.rs`.

### 2e. Slab capacity dinámica

`SlabCache::new()` usa capacity fija de 64 objetos. Calcular
`capacity = PAGE_SIZE / object_size`. Reduce fragmentación interna.

**Archivo:** `memory/heap.rs`.

---

## Bloque 3 — Threads: FD table compartida

**Severidad:** Alta para multi-threading correcto.

`clone(CLONE_VM | CLONE_THREAD)` copia la FD table en vez de compartirla. Un
`close(fd)` en un thread no afecta a los demás.

Fix: envolver `FdTable` en `Arc<SpinLock<FdTable>>`. `fork()` clona el Arc (copia
independiente). `clone_thread()` clona el puntero (tabla compartida).

**Archivos:** `process/mod.rs`, `scheduler/mod.rs`, `syscall/mod.rs`.

---

## Bloque 4 — Syscalls pendientes

**Severidad:** Media. Requeridas para compatibilidad POSIX.

| Syscall | Estado | Notas |
|---|---|---|
| `select()` | ❌ | Implementable como thin wrapper sobre epoll; `fd_set = [u64; 16]` |
| `sendmsg` / `recvmsg` (SCM_RIGHTS) | ❌ | Requerido por D-Bus, Wayland, ssh-agent |
| `chmod` / `chown` / `fchmod` / `fchown` | ❌ | Requiere UID/GID primero |
| `setuid` / `setgid` / `geteuid` / `getegid` | ❌ | Bloque 5 |
| ELF `PT_INTERP` (dynamic linker) | ❌ | Requiere aux vector en stack |
| `getrandom` (pool real) | ⚠️ | Existe el syscall; sin entropy pool real (Bloque 2d) |

**`select()`** es la más urgente — muchos programas POSIX la usan directamente.
Implementación: iterar bits de `fd_set`, llamar `check_fd_readiness()` por cada
bit activo. Reusar lógica de `epoll.rs`.

**SCM_RIGHTS** requiere `msghdr` + `cmsghdr` con array de fds; duplicar cada fd
en la FD table del receptor.

---

## Bloque 5 — Seguridad y permisos

**Severidad:** Alta antes de multi-usuario o producción.

### 5a. UID/GID por proceso

`sys_getuid()` / `sys_getgid()` devuelven 0 siempre. Añadir a `Process`:

```rust
pub uid: u32, pub gid: u32, pub euid: u32, pub egid: u32,
```

Init hereda uid=0. Hijos heredan del padre. `exec()` aplica set-uid bit del ELF.

Nuevos syscalls: `getuid`, `getgid`, `geteuid`, `getegid`, `setuid`, `setgid`.

### 5b. Permisos de archivo en VFS

`InodeStat.mode` existe pero nunca se verifica. Añadir `owner_uid` / `owner_gid`
a `InodeStat`. En `sys_open()`, verificar bits de permiso según `euid`/`egid`.
Root (euid=0) bypasa todo excepto execute sin bit x.

### 5c. copy_from_user / copy_to_user con validación completa

El chequeo actual valida rango pero no verifica que las páginas estén mapeadas.
Añadir walk de page-table antes de dereferenciar punteros de usuario.

### 5d. kill() permission check

`sys_kill()` envía señal a cualquier PID sin restricción. Verificar:
`sender_euid == 0 || sender_uid == target_uid || sender_euid == target_uid`.

### 5e. Binary Permission Model (permissiond)

Ver `docs/features/Binary Permission Model.md`. Requiere 5a–5b como prerequisito.
Deferred post-v1.0.

---

## Bloque 6 — Red (TCP/IP)

**Severidad:** Baja para v1.0, Alta para server workloads.

No hay networking. Implementación completa requiere:

1. **VirtIO-net driver** (`platform/qemu_virt/virtio_net.rs`) — device ID=1,
   features `VIRTIO_NET_F_MAC`, dos virtqueues (RX/TX).
2. **Ethernet + ARP** — dispatch por ethertype, ARP cache (max 64 entradas, LRU).
3. **IPv4 + ICMP** — checksum, dispatch por protocolo, echo reply para `ping`.
4. **UDP** — demux por puerto, checksum con pseudo-header.
5. **TCP** — state machine, retransmission timer, ring buffers 64 KB RX/TX, sliding window.
   Alternativa: integrar `smoltcp` (no_std Rust TCP/IP stack).
6. **BSD socket API** — `socket(AF_INET,...)`, `bind`, `connect`, `listen`,
   `accept`, `send`, `recv`, `setsockopt`.
7. **DHCP client** (userspace) — para obtener IP de QEMU NAT.

---

## Bloque 7 — bzinit y userspace

Deuda de `docs/tech-debt/bzinit-v1.md`.

### 7.0 bzinit: kernel-only invocation

`bzinit` debe ser el único proceso que puede invocarlo el kernel directamente.
Ningún proceso de userspace debe poder hacer `exec("//system:/bin/bzinit")` —
si alguien lo ejecuta manualmente, el kernel debe retornar `EPERM`.

**Mecanismo:** Añadir un flag `INODE_KERNEL_EXEC_ONLY` en el inodo de bzinit
(o en su entrada de política Tier 1). En `sys_exec()`, antes del tier dispatch:

```rust
if inode.flags & INODE_KERNEL_EXEC_ONLY != 0 {
    if !caller_is_kernel_bootstrap() {
        return Err(FsError::PermissionDenied);  // EPERM
    }
}
```

`caller_is_kernel_bootstrap()` es true solo cuando el PID del caller es el
proceso idle (PID 0) durante el bootstrap inicial — es decir, cuando el kernel
mismo ejecuta el primer proceso.

Alternativamente: marcar el inodo de bzinit con un bit especial en `stat().mode`
(`S_ISVTX` extendido o un bit Bazzulto-específico en los bits [63:12]) que el
loader verifica. Este bit solo puede ser seteado por el kernel en `vfs_init()`,
no por ningún syscall de userspace.

**Archivos:** `fs/inode.rs` (flag INODE_KERNEL_EXEC_ONLY), `fs/tmpfs.rs` o
`fs/vfs.rs` (verificación en sys_exec), `fs/mount.rs` (setear el flag en
vfs_init al registrar bzinit).

| Item | Estado | Prerequisito |
|---|---|---|
| `errno` en libc_compat | ❌ | TLS o per-process errno page |
| `bzctl start/stop` via IPC | ❌ | Named pipe o fd de control en /proc/bzinit |
| `bzctl logs <name>` | ❌ | Redirigir stdout/stderr de servicio a pipe |
| `bzctl timeline/graph` | ❌ | Ring buffer de eventos en bzinit |
| Symlinks reales en boot (`/bin` → `/system/bin`) | ✅ | VFS Symlink ya implementado |
| `mkdir/rmdir/rename` en libc_compat | ⚠️ | Stubs — VFS soporta mkdir/unlink, falta rename |
| `execve` con envp en libc_compat | ✅ | envp implementado en kernel |
| Directorios de servicios por usuario | ❌ | readdir en tmpfs |
| Boot snapshot (estado persistente entre reboots) | ❌ | FAT32 write + fsync |
| Dynamic BDL loader | ❌ | ELF PLT/GOT, stable BSL ABI |
| Directory.watch() (inotify) | ❌ | Fuente de eventos de filesystem en kernel |
| App sandbox / capabilities | ❌ | Binary Permission Model (Bloque 5e) |

---

## Bloque 8 — Teclado / Console

**Severidad:** Baja-Media.

| Item | Archivo |
|---|---|
| console_putc() no thread-safe (UTF-8 buffer sin spinlock) | `drivers/console/console.c` |
| Key repeat rate no configurable desde el guest | `platform/qemu_virt/keyboard_virtio.c` |
| Glifos Latin Extended generados programáticamente, sin validación visual | `drivers/console/font_latin_extended.c` |
| keymap_parse() no reporta línea ni razón de error | `drivers/keyboard/keymap.c` |

---

## Bloque 9 — Editor de texto

**Severidad:** Media — requerido para v1.0 usable.

No existe editor de texto en userspace. Mínimo viable: editor modal tipo `vi`
o editor de línea tipo `nano`.

Prerequisitos para nano-like:
- PTY raw mode (ya funciona via `ioctl(TCSETSW)`).
- `TIOCGWINSZ` para obtener dimensiones del terminal (ya implementado).
- `open/read/write/close` sobre VFS (ya funciona).
- Ningún requisito de kernel adicional.

Implementación sugerida: `userspace/bin/edit/` — editor de línea minimalista
con búsqueda, cortar/pegar de líneas, guardado atómico (write a tmp + rename).

---

## Bloque 10 — FD Capabilities (Fuchsia-inspired)

**Severidad:** Baja — post-v1.0.

Añadir campo `rights: u32` a cada file descriptor. Verificar en cada syscall que
use un fd (`read`, `write`, `seek`, `dup`, `send_fd`). `dup()` solo puede reducir
rights, nunca escalar. Ver `docs/FEATURES.md` para diseño completo.

---

## Orden de implementación sugerido

```
Bloque 1: FAT32 completo (BlockDevice + particiones + Fat32Volume por instancia)
    ↓
Bloque 2a: Demand paging lazy BSS
Bloque 2b: MAP_SHARED CoW fix
    ↓
Bloque 3: FD table compartida en threads
    ↓
Bloque 4: select() + SCM_RIGHTS
    ↓
Bloque 5a–5d: UID/GID + permisos de archivo + kill() check
    ↓
Bloque 9: Editor de texto (sin deps de kernel)
    ↓
Bloque 6: Red (VirtIO-net → smoltcp → BSD sockets)
    ↓
Bloque 5e: Binary Permission Model
    ↓
Bloque 10: FD Capabilities
```

Bloque 7 (bzinit) y Bloque 8 (teclado/console) son independientes y pueden
hacerse en cualquier punto del orden anterior.

---

## Deuda resuelta en sesiones recientes

Los siguientes ítems estaban abiertos y fueron completados:

| Item | Sesión |
|---|---|
| SIGALRM / alarm() | 2026-04-12 |
| TLB shootdown (`tlbi vmalle1is`) | 2026-04-12 |
| Reboot / shutdown (PSCI) | 2026-04-12 |
| IPC nlinks discriminante (sem=1, socket=2, mqueue=3) | 2026-04-12 |
| /proc/self symlink | 2026-04-12 |
| procfs por inodos (ProcfsRootInode, ProcPidDirInode) | 2026-04-12 |
| VFS resolución de symlinks (follow_symlinks, MAX_DEPTH=8) | 2026-04-12 |
| InodeType::Symlink + SymlinkInode | 2026-04-12 |
| umask | 2026-04-11 |
| envp en exec | 2026-04-11 |
| sigaltstack + SA_ONSTACK + signal frame save/restore | 2026-04-11 |
| Terminal signals Ctrl+C/Z/\\ (ya estaba implementado, verificado) | 2026-04-12 |
| vDSO clock_gettime acelerado (CNTPCT_EL0, data page 0x3000) | 2026-04-11 |

---

## Post-v1.0 — Deuda documentada para v2.0+

Estas deudas son trabajo real y fueron consideradas para v1.0, pero están explícitamente diferidas post-v1.0 porque requieren userspace estable o componentes que dependen de características aún no implementadas.

### Seguridad / Modelo de permisos (post-v1.0)

**Contexto:** El Binary Permission Model está completamente especificado en `docs/features/Binary Permission Model.md`. En v1.0, se implementa el lado kernel (granted_permissions, vfs_open check, Tier 1/4 parcial). El lado permissiond + Ed25519 + IPC + prompts va post-v1.0.

- **Tier 2 policy store** (`//sys:policy:{sha256}`): requiere permissiond corriendo en userspace. Kernel ya tendrá la infraestructura en v1.0.
- **Tier 3 ELF section + approval prompt**: requiere permissiond + UI terminal/gráfica. Ver paso 7 en `docs/features/Binary Permission Model.md`.
- **Ed25519 Tier 1 signature verification**: requiere clave pública embedded en kernel + herramienta de signing en toolchain. Prerequisito: binarios del sistema firmados.
- **Merkle root computation**: depende de dynamic linker para resolver `.so` deps.
- **`sys_request_cap`** (runtime elevation) + rate limiting: requiere permissiond.
- **`sys_powerbox_open`** (file picker sin exponer path): requiere permissiond + UI.
- **`sys_restrict_self`** (irreversible privilege reduction para interpreters): requiere Tier 2/3.
- **IPC fd re-validation** en `recvmsg` SCM_RIGHTS: requiere Tier 2/3 completos.
- **Password re-prompt** para permisos Authenticated: requiere permissiond + credenciales.
- **TPM binding** para policy entries: deferred hasta hardware real.
- **Saved-set-UID**: `setuid` con semántica POSIX completa (tres UIDs: real, effective, saved).
- **`seccomp` filter**: deferred hasta que modelo de seguridad esté estable.

### Memoria (post-v1.0)

- **Buddy allocator**: el first-fit actual es funcional. Buddy es una optimización de fragmentación para reducir wastage bajo carga real.
- **`vmalloc`**: para objetos grandes del kernel. El slab+first-fit es suficiente para v1.0.
- **Memory hotplug**: añadir RAM en vivo — requiere arquitectura más sofisticada.

### IPC (post-v1.0)

- **`sendmsg` / `recvmsg` SCM_RIGHTS**: requiere fd re-validation contra granted_permissions de receiver. Requiere Binary Permission Model Tier 2/3.
- **`FUTEX_REQUEUE` / `FUTEX_WAIT_BITSET`**: operaciones futex avanzadas para compatibilidad glibc.
- **`CLONE_SIGHAND` / `CLONE_FS` / `CLONE_NEWNS`**: flags de clone avanzados.
- **`splice()` / `vmsplice()`**: zero-copy syscalls para pipes.

### Scheduling (post-v1.0)

- **`Arc<PageTable>`** en clone de threads: actualmente raw pointer documentado como TECHNICAL DEBT. Requiere ownership model mejor.
- **Orphan reparenting completo** a PID 1: parcialmente implementado. Falta garantía de que todos los huérfanos se reasignan.

### Filesystem (post-v1.0)

- **BAFS como filesystem nativo por defecto**: una vez que BAFS v1 esté maduro y testeado en kernel. FAT32 sigue siendo el bootloader/alternativo.
- **FSInfo writeback** en FAT32: el free cluster count no se escribe de vuelta al FSInfo sector. Bajo impacto.
- **`rename` en FAT32 y BAFS**: `Inode::link_child()` retorna `NotSupported` actualmente.
- **Non-ASCII filenames** en FAT32: `lfn_char_to_ascii()` convierte a `?` todo código >= U+0080.
- **Hard links en BAFS**: la V1 del spec no incluye hard links, solo symlinks.
- **Extended attributes** (xattrs): `baz.*` namespace para hints de permisos (post-v1.0).

### Red (explícitamente fuera de scope)

- **TCP/IP stack completo**: VirtIO-net driver, Ethernet, ARP, IPv4, ICMP, UDP, TCP. Explícitamente deferred indefinidamente.
- **DHCP client**: configuración automática de IP.
- **DNS resolver**: IANA zone file o stub resolver.

### Userspace / BDL (no es kernel, pero bloqueante para ciertos features)

- **`permissiond` daemon**: primer binario Tier 1 que debe escribirse en userspace (Rust o C). Requiere scheduler + IPC estables en kernel.
- **Dynamic linker / PT_INTERP**: requiere `mmap` file-backed (MAP_PRIVATE) en kernel (Fase D). Linker muestra símbolos + relocations.
- **Text editor (nano-like)**: BDL/userspace, no kernel.
- **`bzctl` + service manager avanzado**: control de servicio con timeline/dependency graph.
- **Package manager (`baz`)**: ya existe userspace, requiere syscall `grant_permissions()` en kernel (permissiond).
- **Bazzulto Standard Library (BSL)**: API stable para userspace (I/O, memory, permissions, IPC, crypto).

### v1.0 blockers reales — qué debe estar hecho

Para declarar v1.0, el kernel debe tener:

1. ✅ FAT32 per-volume (Fase A)
2. ✅ Demand paging + MAP_SHARED file-backed (Fases C, D)
3. ✅ UIDs/GIDs POSIX (Fase E)
4. ✅ Binary Permission Model kernel side — granted_permissions + vfs_open check (Fase E2)
5. ✅ fsync + VMA tree + ASLR (Fase F)
6. ✅ TLB shootdown SMP fix (Fase B)
7. ✅ Stubs menores filled in (Fase G)
8. ✅ BAFS driver opcional soportado (Fase G)
9. ✅ docs/DEBT.md actualizado con deuda post-v1.0

Una vez hecho, el kernel es "estable v1.0". Ninguno de los post-v1.0 items anterior bloquea ese hito.
