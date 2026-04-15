# Bazzulto OS — Roadmap to v2.0

> Este documento asume que v1.0 está completamente entregado y taggeado.
> Todo lo que se lista aquí depende de la base estable de v1.0.

---

## Lo que v1.0 entrega (base de partida)

- Kernel AArch64 estable, UEFI/Limine, QEMU virt + VirtualBox
- POSIX identity (UID/GID/DAC) + Binary Permission Model Tiers 1, 2, 4
- Btrfs como root filesystem (BAFS disponible como secundario opcional)
- 162 syscalls estables (ABI frozen)
- musl libc estática + compartida (`libc.so.6`)
- Dynamic linker (`ld-bazzulto.so.1`) + `libbsl.so.1`
- bzsh POSIX §2 completo, terminal xterm-256color
- ~57 coreutils, kibi (editor), boot splash, font system
- bzinit con 6 servicios supervisados
- BSL 1.0 frozen (System, IO, Display, Concurrency, Diagnostics)
- **Sin red. Sin compilador. Sin GUI. Un solo core.**

---

## Tema de v2.0: "Conectado y Construible"

v2.0 convierte Bazzulto en un OS donde puedes:
- Conectarte a una red y hacer cosas útiles con ella
- Escribir y compilar programas directamente en el sistema
- Instalar software nuevo sin recompilar el disco imagen
- Correr en hardware con múltiples cores

---

## Lo que v2.0 Explícitamente Excluye

- GUI / compositor / windowing system (v3.0)
- Soporte de audio (v3.0)
- Soporte USB completo (post-v2.0)
- Rust compiler (`rustc`) — demasiado complejo aún
- GCC / Clang — demasiado complejo aún
- LLVM como backend — post-v2.0
- Debugger simbólico (gdb-like) — post-v2.0
- Contenedores / namespaces — post-v2.0
- Virtualización (KVM) — post-v2.0
- Filesystem encryption — post-v2.0
- Bluetooth stack — post-v2.0

---

## Milestone Map

| # | Milestone | Version Tag |
|---|-----------|-------------|
| N1 | VirtIO-net driver + stack TCP/IP (smoltcp) | v1.1 |
| N2 | BSD socket API para AF_INET/AF_INET6 | v1.2 |
| N3 | DHCP client + DNS resolver + herramientas de red básicas | v1.3 |
| N4 | TCC compiler + assembler + ar | v1.4 |
| N5 | libstdc++ mínima + C++ compilation | v1.5 |
| N6 | Package manager (baz) | v1.6 |
| N7 | SMP — arranque multi-core + scheduler | v1.7 |
| N8 | Binary Permission Model Tier 3 | v1.8 |
| N9 | Security hardening (seccomp, Ed25519, sys_request_cap) | v1.9 |
| N10 | BSL v2.0 (Crypto, Net, Storage) + API freeze | v2.0-rc |
| N11 | Integración, testing y release v2.0 | v2.0 |

**Dependencias:**
```
N1 → N2 → N3
N3 → N6 (package manager usa red para repositorio)
N4 → N5 → N6 (package manager compila desde fuente)
N6 → N8 (baz lee sección ELF en install time)
N7 — independiente, puede ir en paralelo con N1-N6
N8 → N9
N1, N4 → N10 (BSL.Net necesita stack, BSL.Crypto necesita primitivas)
```

---

## N1 — VirtIO-net Driver + Stack TCP/IP (→ v1.1)

**Goal:** El kernel tiene un driver VirtIO-net funcional. La pila TCP/IP está integrada usando smoltcp. El sistema puede enviar y recibir paquetes en la red de QEMU.

**Por qué smoltcp:** Es Rust puro, no_std, mantenido activamente, tiene soporte VirtIO-net, e implementa TCP, UDP, ICMP, ARP, IPv4, IPv6. El kernel ya tiene el patrón VirtIO (block device en `kernel/src/platform/virtio_block.rs`) — el driver de red sigue la misma estructura VirtIO MMIO.

### Tasks

**N1.1 — VirtIO-net device driver**

Crear `kernel/src/platform/virtio_net.rs`. Siguiendo el patrón de `virtio_block.rs`:
- Detectar dispositivo VirtIO-net en el MMIO window (`0x0A000000` + offset según DTB)
- Inicializar virtqueues: receive queue (RX) y transmit queue (TX)
- Implementar `send_packet(buf: &[u8])` y `receive_packet(buf: &mut [u8]) -> usize`
- Registrar IRQ en GIC para notificaciones RX
- Leer MAC address del dispositivo desde los feature bits de VirtIO

**N1.2 — Integrar smoltcp**

Añadir smoltcp como dependencia en `kernel/Cargo.toml` (con `default-features = false`, solo features `tcp`, `udp`, `icmp`, `medium-ethernet`, `proto-ipv4`). Crear `kernel/src/net/mod.rs` como capa de interfaz entre smoltcp y el driver VirtIO-net.

Implementar el trait `smoltcp::phy::Device` sobre el driver VirtIO-net: `transmit()` pasa el buffer al TX queue, `receive()` drena el RX queue.

**N1.3 — Interfaz de red en kernel**

Crear `kernel/src/net/interface.rs`. Una interfaz de red global con:
- Dirección IP configurable (por DHCP en N3, o estática hardcodeada para N1)
- Default gateway
- MTU: 1500
- Polling de smoltcp integrado en el timer tick del scheduler

**N1.4 — Socket interno del kernel**

Crear la abstracción `kernel/src/net/socket.rs`: un socket TCP/UDP interno que el resto del kernel puede usar. Esto es la capa sobre la que el syscall layer construirá `AF_INET` en N2.

**N1.5 — Test básico**

Añadir en QEMU: `-netdev user,id=net0 -device virtio-net-pci,netdev=net0`. En el kernel, implementar un ping ICMP echo reply hardcodeado. Verificar con `ping` desde el host que el kernel responde.

**N1.6** Tag commit `v1.1-virtio-net`.

### Exit criteria
QEMU arranca con VirtIO-net. El kernel responde a ping ICMP desde el host. El driver no produce kernel panic bajo carga de paquetes.

---

## N2 — BSD Socket API para AF_INET/AF_INET6 (→ v1.2)

**Goal:** Los syscalls de socket existentes (ya en la tabla desde v1.0) son implementados completamente para `AF_INET` y `AF_INET6`. Los programas en userspace pueden abrir conexiones TCP y UDP.

### Tasks

**N2.1 — Implementar `sys_socket(AF_INET)` y `sys_socket(AF_INET6)`**

Los syscalls `socket(79)`, `bind(80)`, `listen(81)`, `accept(82)`, `connect(83)`, `send(84)`, `recv(85)` ya existen en la tabla ABI de v1.0 pero solo implementaban `AF_UNIX`. Extender los handlers en `kernel/src/systemcalls/` para `AF_INET` y `AF_INET6`:
- `socket(AF_INET, SOCK_STREAM, 0)` → crea un socket TCP sobre smoltcp
- `socket(AF_INET, SOCK_DGRAM, 0)` → crea un socket UDP sobre smoltcp
- `bind()` → asigna dirección IP + puerto local
- `connect()` → inicia handshake TCP, bloquea hasta establecido (o error)
- `listen()` + `accept()` → modo servidor TCP, accept bloquea en wait queue
- `send()` / `recv()` → escritura/lectura sobre TCP stream o UDP datagram
- `sendto()` / `recvfrom()` → UDP con dirección explícita (añadir syscalls si no están)

**N2.2 — `getaddrinfo` en musl**

musl incluye `getaddrinfo()`. Verificar que resuelve correctamente con el DNS resolver que se añadirá en N3. Para N2, aceptar que solo funciona con IPs literales (sin DNS todavía).

**N2.3 — `setsockopt` / `getsockopt`**

Añadir `setsockopt(140)` y `getsockopt(141)` a la tabla ABI (`[PROVISIONAL]`). Implementar: `SO_REUSEADDR`, `SO_REUSEPORT`, `TCP_NODELAY`, `SO_KEEPALIVE`, `SO_RCVTIMEO`, `SO_SNDTIMEO`.

**N2.4 — `epoll` con sockets de red**

Verificar que `epoll_ctl(fd)` funciona con fds de socket TCP/UDP. El mecanismo de readiness notification de smoltcp debe conectarse con el estado del epoll set en el kernel.

**N2.5 — Test: TCP echo server**

Crear `tests/net/tcp_echo.c`: escucha en puerto 7 (echo), acepta conexiones, hace eco de cada línea. Verificar desde el host con `nc 127.0.0.1 7`.

**N2.6 — Bazzulto.Net (stub activado)**

Activar el módulo `Bazzulto.Net` en BSL (estaba desactivado en v1.0). API inicial: `TcpStream::connect(addr)`, `TcpListener::bind(addr)`, `UdpSocket::bind(addr)`. Implementado sobre los syscalls de socket.

**N2.7** Tag commit `v1.2-inet-sockets`.

### Exit criteria
`curl http://1.1.1.1/` (con IP literal) hace una petición HTTP y muestra la respuesta. TCP echo server funciona desde el host.

---

## N3 — DHCP + DNS + Herramientas de Red (→ v1.3)

**Goal:** El sistema se configura automáticamente en red vía DHCP. Los nombres de dominio se resuelven. Las herramientas de red básicas están disponibles.

### Tasks

**N3.1 — Cliente DHCP**

Crear `userspace/services/bzdhcpd/` — un daemon que:
- Al arranque envía DHCPDISCOVER por broadcast (usando el socket UDP raw o `AF_PACKET`)
- Procesa DHCPOFFER, envía DHCPREQUEST, procesa DHCPACK
- Configura la interfaz de red: IP, máscara, gateway, DNS servers
- Escribe la configuración recibida en `/system/etc/network/dhcp-lease`
- Renueva el lease antes de que expire (T1/T2 timers)
- Restart policy: `always` — si cae, el sistema queda sin red

**N3.2 — Configuración de red en kernel**

Añadir `sys_ifconfig(name, ip, mask, gateway)` (nuevo syscall `[PROVISIONAL]`). `bzdhcpd` lo llama tras recibir el DHCPACK. La interfaz smoltcp se actualiza con los nuevos valores.

**N3.3 — DNS resolver**

musl incluye un resolver DNS iterativo que lee `/system/etc/resolv.conf`. Verificar que funciona correctamente con la configuración que escribe `bzdhcpd`. Crear `/system/etc/resolv.conf` con formato estándar (`nameserver x.x.x.x`).

Crear también `/system/etc/hosts` con entradas básicas:
```
127.0.0.1  localhost
::1        localhost
```

**N3.4 — Herramientas de red básicas (coreutils de red)**

Añadir a `/system/bin/`:
- `ping` — ICMP echo request/reply, opciones `-c count`, `-i interval`
- `ifconfig` — mostrar y configurar interfaces de red (sin argumentos: lista todas)
- `netstat` — conexiones activas, puertos en escucha (`-t` TCP, `-u` UDP, `-l` listening)
- `wget` — descarga HTTP/HTTPS. Para v2: solo HTTP (sin TLS). Un archivo a la vez.
- `nc` (netcat) — conectar TCP/UDP en modo cliente y servidor
- `host` — resolución DNS simple: `host google.com`

**N3.5 — bzinit: arranque de red**

Añadir `bzdhcpd` al boot graph de bzinit:
```
Level 1 (tras bzlogd): permissiond, bzdhcpd
```
`bzdhcpd` es opcional (si no hay interfaz de red, falla silenciosamente sin bloquear el boot).

**N3.6 — `bzsplash` progress para red**

Añadir fase en bzsplash: `"Configuring network..."` (bzdhcpd).

**N3.7** Tag commit `v1.3-networking`.

### Exit criteria
QEMU arranca con `-netdev user` y el sistema obtiene IP por DHCP. `ping 1.1.1.1` funciona. `host google.com` resuelve. `wget http://example.com/` descarga el archivo.

---

## N4 — TCC Compiler + Assembler + ar (→ v1.4)

**Goal:** Un programador puede escribir y compilar programas C directamente en Bazzulto OS. `tcc` compila C99 y produce ELF AArch64 que corre correctamente en el sistema.

### Tasks

**N4.1 — Compilar TCC para aarch64-bazzulto**

Obtener TCC (mob branch, que incluye soporte AArch64 más reciente). Compilar para `aarch64` contra musl libc. Configurar el sysroot:
```
tcc -B /system/lib -I /system/include -L /system/lib
```
TCC usa sus propios headers internos para `stddef.h`, `stdarg.h`, `stdint.h`, etc. Verificar que no colisionen con los headers de musl en `/system/include/`.

**N4.2 — Configuración del entorno TCC**

Crear `/system/etc/tcc/` con la configuración del compilador:
```
# /system/etc/tcc/tcc.conf
sysincludepaths = ["/system/include"]
libpaths = ["/system/lib"]
crt_prefix = "/system/lib"
```

Crear `/system/lib/crt1.o`, `/system/lib/crti.o`, `/system/lib/crtn.o` — los objetos de startup de musl, necesarios para `tcc -run` y para linking estático.

**N4.3 — Verificar ABI AArch64**

TCC debe generar código que siga el AArch64 ABI:
- Calling convention: parámetros en x0-x7, resultado en x0, callee-saved x19-x28
- Stack alignment: 16 bytes en punto de llamada
- Relocations: `R_AARCH64_CALL26`, `R_AARCH64_ADR_PREL_PG_HI21`, `R_AARCH64_ADD_ABS_LO12_NC`

Verificar que los binarios producidos por TCC son cargables por el ELF loader de Bazzulto y corren correctamente.

**N4.4 — `tcc -run` mode**

TCC tiene un modo de ejecución directa: `tcc -run programa.c` compila en memoria y ejecuta sin generar un archivo. Verificar que funciona en Bazzulto — es muy útil para scripting con C.

**N4.5 — Instalar TCC**

Añadir a `disk.img`:
- `/system/bin/tcc` — el compilador
- `/system/lib/libtcc.so` — la librería de TCC (para uso embebido)
- `/system/include/tcclib.h` — header para usar libtcc desde otro programa

**N4.6 — `as` — assembler AArch64**

TCC incluye un assembler interno pero para portabilidad añadir `as` (GNU assembler de binutils) como binario independiente compilado contra musl. Instalar en `/system/bin/as`.

**N4.7 — `ar` — archiver**

Necesario para crear archivos `.a` (librerías estáticas). Añadir `ar` (de binutils o implementación standalone) a `/system/bin/ar`.

**N4.8 — `make`**

GNU make compilado contra musl. Necesario para construir proyectos con Makefiles. Instalar en `/system/bin/make`.

**N4.9 — Test suite del compilador**

Crear `tests/compiler/`:
- `test_hello.c`: `printf("hello\n"); return 0;` — compilar y correr
- `test_structs.c`: structs, punteros, arrays — verificar layout correcto
- `test_dynamic.c`: compilar dinámicamente, linkear contra `libc.so.6` y `libbsl.so.1`
- `test_selfhost.c`: compilar un programa que use `libtcc` para compilar otro programa en runtime

**N4.10** Tag commit `v1.4-tcc-compiler`.

### Exit criteria
`echo '#include <stdio.h>\nint main(){printf("ok\\n");}' | tcc -run -` imprime `ok`. `tcc -o hello hello.c && ./hello` produce un binario que corre. `make` ejecuta un Makefile básico.

---

## N5 — libstdc++ Mínima + Compilación C++ (→ v1.5)

**Goal:** Programas C++ básicos compilan y corren en Bazzulto. La stdlib C++ cubre las partes más usadas.

**Nota de scope:** Una libstdc++ completa (GCC runtime) o libc++ completa (LLVM) son enormes. Para v2.0, el objetivo es una librería C++ funcional para los casos de uso más comunes, no una implementación al 100% del estándar C++23.

### Tasks

**N5.1 — Evaluar opciones de C++ stdlib**

Comparar para Bazzulto:
- `libc++` de LLVM con `libunwind` + `libc++abi`: la opción más modular, separable de LLVM
- `uClibc++`: C++ stdlib minimalista para sistemas embebidos, ~50K LOC
- `llvm-libcxx` con musl: combinación conocida en Alpine Linux

Decisión: usar **uClibc++** para v2.0. Tiene dependencias mínimas, soporta AArch64, y es suficiente para los casos de uso v2.0. Una migración a libc++ completa es post-v2.0.

**N5.2 — Compilar uClibc++ para aarch64-bazzulto**

Compilar contra musl libc. Producir `libstdc++.so.6` (con SONAME `libstdc++.so.6` para compatibilidad) y `libstdc++.a`. Instalar en `/system/lib/`.

**N5.3 — C++ runtime support**

Añadir soporte básico de:
- `new` / `delete` operadores
- RTTI básico (typeinfo, dynamic_cast)
- Exception handling con `libunwind` (o un fallback de terminate-on-exception para v2.0)
- Inicialización de variables globales estáticas (`.init_array`)

**N5.4 — Headers C++ de BSL**

Completar los headers `.hpp` de BSL en `include/bazzulto/`. Ahora que existe una stdlib C++, los headers pueden exponer clases C++ (no solo C ABI). Añadir wrappers C++ para `Bazzulto.IO::File`, `Bazzulto.Diagnostics`, etc.

**N5.5 — TCC + C++**

TCC tiene soporte C++ experimental. Verificar hasta dónde llega en Bazzulto o documentar que para C++ se requiere un compilador externo (post-v2.0 cuando llegue GCC/Clang).

Alternativa: documentar que C++ se compila cruzado en el host y se copia al sistema, hasta que haya un compilador C++ nativo.

**N5.6** Tag commit `v1.5-cpp-stdlib`.

### Exit criteria
`tcc -run test.cpp` (C++ básico sin templates complejos) funciona. Un programa que usa `std::string`, `std::vector` y `std::cout` compila y corre. `new`/`delete` no leak.

---

## N6 — Package Manager: baz (→ v1.6)

**Goal:** `baz install`, `baz remove`, `baz list`, `baz search` funcionan. Los paquetes se instalan desde un repositorio HTTP. La integración con BPM lanza el dialog de permisos al instalar.

### Tasks

**N6.1 — Formato de paquete Bazzulto**

Definir el formato `.bpkg` (Bazzulto Package):
- Archivo tar.zst (zstd para compresión, implementar zstd descompresión o usar una librería)
- Contenido:
  ```
  manifest.toml       # metadatos: name, version, deps, description, files
  files/              # árbol de archivos a instalar
  scripts/
    pre_install.sh    # opcional
    post_install.sh   # opcional
  permissions.bpm     # capabilities que el paquete solicita (para BPM Tier 3)
  ```
- `manifest.toml` formato:
  ```toml
  name = "hello"
  version = "1.0.0"
  description = "A hello world package"
  depends = ["libc >= 1.0"]
  install_prefix = "/system"
  ```

**N6.2 — Repositorio de paquetes**

Definir el protocolo de repositorio:
- Índice en `https://repo.bazzulto.dev/index.bpkg-index` (JSON: lista de paquetes con name, version, sha256, url)
- Paquetes en `https://repo.bazzulto.dev/packages/<name>-<version>.bpkg`
- Para v2.0: también soporte de repositorio local (carpeta en `/system/packages/local/`)

**N6.3 — `baz` — el CLI**

Crear `userspace/src/baz/`. Comandos:

- `baz install <package>`: descarga `.bpkg`, verifica sha256, descomprime en staging, lee `permissions.bpm` y lanza dialog BPM Tier 3 (N8), si aprobado instala los archivos, registra en la base de datos local
- `baz remove <package>`: elimina los archivos registrados, actualiza la base de datos
- `baz update`: actualiza el índice del repositorio
- `baz upgrade`: actualiza paquetes instalados que tienen nueva versión disponible
- `baz list`: lista paquetes instalados con versión
- `baz search <query>`: busca en el índice
- `baz info <package>`: muestra metadatos del paquete

**N6.4 — Base de datos local de paquetes**

`/system/var/baz/installed.db` — BAFS, formato simple: un directorio por paquete instalado con el manifest y el listado de archivos. `baz` usa el VFS directamente; no necesita un daemon.

**N6.5 — Integración con BPM Tier 3**

`baz install` lee `permissions.bpm` del paquete antes de instalarlo. Para cada binario en el paquete que declare capabilities:
- Si el usuario está en terminal interactivo: mostrar el dialog de Tier 3 (N8) con la lista de capabilities solicitadas
- Si el usuario acepta: añadir entrada en `/system/policy/` para ese binario
- Si rechaza: los binarios se instalan pero sin capabilities declaradas (Tier 4 al ejecutar)

**N6.6 — `baz build` (compilación desde fuente)**

`baz build <build_dir>`: busca un `bazzulto.build` en el directorio (Makefile o script), lo ejecuta con `tcc` / `make`, empaqueta el resultado en un `.bpkg` local. Permite a los desarrolladores crear paquetes sin infraestructura de repositorio.

**N6.7** Tag commit `v1.6-package-manager`.

### Exit criteria
`baz install hello` descarga, verifica e instala el paquete. `hello` corre. `baz remove hello` elimina los archivos. `baz search editor` muestra resultados del índice.

---

## N7 — SMP: Multi-core (→ v1.7)

**Goal:** El kernel arranca todos los CPUs disponibles en QEMU virt. El scheduler distribuye procesos entre cores. Los locks son correctos bajo concurrencia real.

### Tasks

**N7.1 — Secondary CPU startup (PSCI)**

En `kernel/src/smp/`, completar el arranque de CPUs secundarios. QEMU virt usa PSCI (Power State Coordination Interface) para encender CPUs adicionales. Implementar `psci_cpu_on(cpu_id, entry_point, context)` llamando al PSCI firmware mediante `HVC` o `SMC` (según la configuración de QEMU). El entry point en los CPUs secundarios ejecuta: configurar stack, habilitar MMU con la page table del kernel, inicializar GIC CPU interface, llamar `secondary_cpu_main(cpu_id)`.

**N7.2 — Per-CPU data structures**

Crear `kernel/src/smp/percpu.rs`. Cada CPU tiene: su propia run queue (`VecDeque<Pid>`), contador de ticks, referencia al proceso actualmente en ejecución, stack de interrupción. Acceso via `TPIDR_EL1` (thread pointer de EL1) que apunta al bloque per-CPU del core actual.

**N7.3 — Scheduler multi-core**

Extender el scheduler en `kernel/src/scheduler/` para distribuir procesos:
- Load balancing: cuando una run queue está vacía, robar trabajo de otra (work stealing)
- Process affinity: opción de pintar un proceso a un CPU específico (`sched_setaffinity`)
- El timer de cada CPU genera su propio tick → cada CPU puede preemptar su propio proceso

**N7.4 — Spinlocks y barreras de memoria**

Auditar todos los spinlocks en el kernel. Bajo SMP, un spinlock que usa `cmpxchg` con ordering incorrecto produce races. Cambiar todos los `Ordering::Relaxed` en operaciones de lock por `Ordering::Acquire`/`Ordering::Release`. Añadir `DMB SY` (Data Memory Barrier) donde sea necesario según el ARM Memory Model.

**N7.5 — IPI (Inter-Processor Interrupts)**

Implementar IPIs via GIC SGI (Software Generated Interrupts). Casos de uso:
- TLB shootdown: cuando un proceso modifica su page table en un CPU, los demás deben invalidar sus TLB entries para ese proceso
- Scheduler kick: despertar un CPU idle cuando llega trabajo nuevo
- Panic broadcast: en kernel panic, detener todos los CPUs (halt)

**N7.6 — QEMU virt con múltiples CPUs**

Actualizar el comando de QEMU en el Makefile: añadir `-smp 4`. Verificar arranque con 4 cores. Añadir test de stress: 4 procesos corriendo en paralelo, cada uno ejecutando operaciones de filesystem y memoria, verificar que no hay deadlock ni corrupción de datos.

**N7.7** Tag commit `v1.7-smp`.

### Exit criteria
QEMU arranca con `-smp 4` y los 4 cores están activos. `cat /proc/cpuinfo` lista 4 procesadores. Un programa con 4 threads corriendo en paralelo termina correctamente. `fsck.bafs` reporta clean después del stress test multi-core.

---

## N8 — Binary Permission Model Tier 3 (→ v1.8)

**Goal:** Los binarios que declaran sus permisos via sección ELF `.bazzulto_permissions` muestran un dialog específico al usuario indicando exactamente qué namespaces necesitan. El usuario decide con información completa.

### Tasks

**N8.1 — Parsear sección ELF `.bazzulto_permissions`**

En `kernel/src/loader/`, después de cargar el ELF: buscar una sección con nombre `.bazzulto_permissions`. Su contenido es una lista de strings null-terminated, cada uno un namespace pattern (`//user:/home/**`, `//dev:cam:0`, etc.). Extraer esta lista y pasarla al query de `permissiond` junto con el hash.

**N8.2 — `permissiond` Tier 3 handler**

En `permissiond`, cuando `query()` recibe un binario con permisos declarados (`TIER3_DECLARED`):

Si TTY disponible, mostrar el dialog en terminal:
```
[bazzulto] /home/user/myapp solicita acceso a:

  • //user:/home/**          leer y escribir tus archivos
  • //dev:cam:0             cámara (requiere autenticación)

  [P]ermanente  [S]esión  [U]na vez  [N]unca:
```

Las descripciones legibles de cada namespace se generan desde una tabla en `permissiond` (`//dev:cam:**` → `"cámara"`).

**N8.3 — Niveles de aprobación diferenciados**

Según la tabla de sensibilidad del BPM (ya definida en `kernel/src/permission/mod.rs`):
- `Consent` namespaces: `[P]ermanente / [S]esión / [U]na vez / [N]unca`
- `Authenticated` namespaces: requiere contraseña del usuario antes de mostrar las opciones
- `Impossible` namespaces: rechazados siempre, no se muestra al usuario (pero se indica que el binario los solicitó)

**N8.4 — Persistencia de aprobaciones**

- `Permanente`: escribe entrada en `/system/policy/<hash>` con scope=permanent
- `Sesión`: mantiene en memoria en `permissiond`, se pierde al reboot
- `Una vez`: permite esta ejecución, no persiste
- `Nunca`: escribe entrada con `DENIED` en `/system/policy/<hash>`

**N8.5 — Toolchain support: declarar permisos en Rust y C**

Crear `userspace/libs/bpm_declare/` — una macro/utilidad para declarar permisos en el código:

En Rust:
```rust
bazzulto_permissions! {
    "//user:/home/**",
    "//ram:/tmp/**",
}
```

En C:
```c
BAZZULTO_PERMISSIONS("//user:/home/**", "//ram:/tmp/**");
```

Estas macros generan la sección ELF `.bazzulto_permissions` en el binario resultante. Documentar en `docs/abi/permission-model.md`.

**N8.6 — Actualizar `baz install`**

`baz install` lee `permissions.bpm` del paquete (que contiene la misma información que la sección ELF) y lanza el mismo dialog antes de instalar. Si el usuario aprueba en install time, no se vuelve a preguntar al ejecutar (Tier 2 ya tiene la entrada).

**N8.7** Tag commit `v1.8-bpm-tier3`.

### Exit criteria
Un binario con `.bazzulto_permissions` muestra el dialog correcto al ejecutar la primera vez. `Permanente` no vuelve a preguntar en reboots. `Nunca` produce EPERM. La macro de Rust genera la sección ELF correctamente verificable con `readelf -S`.

---

## N9 — Security Hardening (→ v1.9)

**Goal:** Completar el modelo de seguridad con las piezas de hardening que quedaron fuera de v1.0.

### Tasks

**N9.1 — Ed25519 signature verification en runtime (BPM Tier 4 completo)**

En `kernel/src/loader/`, para binarios en `//system:/bin/`: verificar la firma Ed25519 del ELF antes de exec (ya definido en el BPM document). La clave pública del sistema está embebida en el kernel. El signing se hace en build time via `bzsign` (herramienta host). Sin firma válida: `EPERM` con mensaje claro.

Actualizar el Makefile para que `bzsign` se aplique automáticamente a todos los binarios del sistema durante el build.

**N9.2 — `sys_restrict_self()`**

Nuevo syscall `[STABLE]`. Permite a un proceso reducir su propio conjunto de capabilities irreversiblemente. Uso principal: intérpretes (bzsh, python) que van a ejecutar código de usuario y quieren limitar lo que ese código puede hacer. Llamado antes de `exec()` del script. Implementar en `kernel/src/systemcalls/`: intersectar `process.granted_permissions` con el mask pasado, sin posibilidad de revertir.

**N9.3 — `sys_request_cap()`**

Nuevo syscall `[PROVISIONAL]`. Permite a un proceso solicitar en runtime una capability adicional. Flujo: kernel envía request a `permissiond` → `permissiond` muestra dialog si TTY disponible → si aprueba, kernel añade la capability al proceso. Rate limiting: 3 denegaciones en 60 s → no más prompts en esa sesión. Implementar en `kernel/src/systemcalls/`.

**N9.4 — seccomp básico**

Implementar `sys_seccomp(SECCOMP_SET_MODE_STRICT)`: restringe el proceso a solo poder llamar `read`, `write`, `_exit`, `sigreturn`. Útil para sandboxes. No implementar BPF filtering completo (post-v2.0) — solo el modo strict.

**N9.5 — `copy_from_user` con protección TOCTOU**

Revisitar `copy_from_user` de M2: en sistemas SMP (N7), un race condition entre la validación del puntero y la copia es posible (TOCTOU). Solución: copiar primero, luego validar la copia (o usar instrucciones atómicas de acceso a memoria de usuario). Documentar la solución en `docs/abi/security.md`.

**N9.6 — Kernel ASLR para shared libraries**

Verificar que en SMP el pool de entropía ASLR es thread-safe (el LFSR necesita un spinlock en N7). Aumentar la entropía: en arranque de cada CPU secundario, XOR el pool con `CNTPCT_EL0` del CPU recién arrancado.

**N9.7 — Audit log de seguridad**

En `permissiond`, loguear en un archivo separado `/data/logs/security.log` todos los eventos de seguridad: exec con Tier 3 (qué aprobó el usuario), sys_request_cap (aprobado/denegado), EPERM por BPM, binarios con firma inválida. El log de seguridad no se mezcla con el log general de `bzlogd`.

**N9.8** Tag commit `v1.9-security-hardening`.

### Exit criteria
Un binario en `/system/bin/` sin firma válida produce EPERM. `sys_restrict_self()` es irreversible. `seccomp STRICT` bloquea syscalls no permitidos. El audit log registra eventos de BPM.

---

## N10 — BSL v2.0: Crypto, Net, Storage (→ v2.0-rc)

**Goal:** Los tres módulos BSL que quedaron fuera de v1.0 están completos, documentados y frozen en v2.0.

### Tasks

**N10.1 — `Bazzulto.Crypto`**

Primitivas criptográficas en Rust puro (sin dependencias de crates externos para las primitivas core):

- **Hash**: SHA-256, SHA-512, BLAKE3
- **MAC**: HMAC-SHA256
- **Symmetric**: AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305
- **Asymmetric**: Ed25519 (sign/verify), X25519 (ECDH key exchange)
- **KDF**: PBKDF2-HMAC-SHA256, HKDF
- **RNG**: `Bazzulto.Crypto.SecureRandom` — lee del ASLR entropy pool del kernel via un nuevo device `/dev/random`
- API: `bzl_sha256(input) -> [u8; 32]`, `bzl_aes_gcm_encrypt(key, nonce, plaintext, aad) -> Vec<u8>`, etc.

Implementar `/dev/random` y `/dev/urandom` en devfs: leen del pool de entropía del kernel (extendido con eventos de hardware: interrupciones, lecturas de RTC, etc.).

**N10.2 — `Bazzulto.Net` (completo)**

Construido sobre los syscalls AF_INET de N2:

- `TcpStream`: connect, read, write, shutdown, set_timeout
- `TcpListener`: bind, accept, incoming iterator
- `UdpSocket`: bind, send_to, recv_from
- `DnsResolver`: resolve (usa `getaddrinfo` de musl)
- `HttpClient` (minimal): GET y POST sin TLS para v2.0. TLS (via Bazzulto.Crypto) en v2.1.
- `SocketAddr`, `IpAddr`, `Ipv4Addr`, `Ipv6Addr` tipos

**N10.3 — `Bazzulto.Storage`**

Abstracciones de alto nivel sobre el VFS:

- `BlockDevice`: lectura/escritura de sectores sobre `/dev/disk*`
- `Partition`: acceso a particiones MBR/GPT
- `VolumeInfo`: tamaño, espacio libre, filesystem type (llama `sys_statfs`)
- `DiskWatcher`: notificaciones de mount/unmount via poll sobre `/proc/mounts`

**N10.4 — Freeze BSL v2.0**

Congelar las APIs de Bazzulto.Crypto, Bazzulto.Net, Bazzulto.Storage en versión `2.0.0`. El proceso es el mismo que en M13 (v1.0): doc completo, sin `todo!()`, tests de integración. Actualizar BSL version a `"2.0.0"`.

**N10.5** Tag commit `v2.0-rc-bsl-freeze`.

### Exit criteria
Los tres módulos tienen APIs documentadas y frozen. Tests de integración pasan. `Bazzulto.Crypto::sha256` produce el hash correcto en vectores de test conocidos. `Bazzulto.Net::TcpStream::connect("example.com:80")` establece una conexión.

---

## N11 — Integración, Testing y Release v2.0

**Goal:** El sistema completo pasa la suite de tests extendida. El ISO v2.0 incluye todas las funcionalidades de v2.0 y arranca en QEMU y VirtualBox.

### Tasks

**N11.1 — Test suite extendida**

Añadir a `tests/v2.0/`:

- `test_network.sh`: ping, wget HTTP, TCP echo, DNS resolution
- `test_compiler.sh`: compilar y correr hello world con TCC; compilar un programa que usa structs y punteros
- `test_packages.sh`: instalar un paquete de prueba desde repositorio local, verificar archivos, desinstalar
- `test_smp.sh`: lanzar 4 procesos paralelos con operaciones de escritura en BAFS, verificar consistencia
- `test_bpm_tier3.sh`: ejecutar un binario con `.bazzulto_permissions`, verificar que el dialog aparece en TTY, simular respuesta `N` y verificar EPERM
- `test_crypto.sh`: verificar SHA-256, AES-GCM, Ed25519 contra vectores de test conocidos
- `test_seccomp.sh`: proceso con seccomp strict solo puede llamar read/write/exit

**N11.2 — ISO v2.0**

Actualizar el Makefile: `make iso` produce `bazzulto-2.0.iso`. El disco imagen incluye:
- TCC + make + ar
- uClibc++
- herramientas de red (ping, wget, nc, netstat, host)
- baz (package manager)
- `/system/include/` con headers musl + BSL v2

**N11.3 — VirtualBox con red**

VirtualBox 7.x: añadir un adaptador de red NAT a la VM. Verificar que el sistema obtiene IP por DHCP y puede hacer ping y wget desde dentro.

**N11.4 — Release notes v2.0**

Crear `docs/release/v2.0.md`: features nuevas respecto a v1.0, instrucciones de red para QEMU y VirtualBox, instrucciones de uso de TCC y baz.

**N11.5** Tag commit `v2.0`.

### Exit criteria
`tests/run_all_v2.sh` pasa en el ISO build. Red funciona en QEMU y VirtualBox. TCC compila un programa C en el sistema. `baz install` funciona.

---

## v2.0 Definition of Done

- [ ] Milestones N1–N11 taggeados en git history
- [ ] `tests/run_all_v2.sh` exits 0 sobre el ISO build
- [ ] Red funciona en QEMU (`-netdev user`) y VirtualBox 7.x (NAT)
- [ ] `tcc -run hello.c` compila y corre directamente en el sistema
- [ ] `baz install <paquete>` instala desde repositorio local
- [ ] SMP con 4 cores estable bajo stress test
- [ ] BPM Tier 3 muestra dialog con capabilities específicas
- [ ] BSL version es `"2.0.0"` con las tres APIs frozen
- [ ] `docs/release/v2.0.md` escrito y preciso
- [ ] `bazzulto-2.0.iso` arranca en QEMU y VirtualBox con red

---

## Lo que queda para v3.0

Estas son las áreas naturales de v3.0, no comprometidas aquí:

- **GUI / Compositor / Windowing system** — el salto más grande
- **GCC o Clang nativo** — compilador de producción en el sistema
- **Debugger** — gdb-like o debugger nativo Bazzulto
- **Audio** — driver VirtIO-sound, BSL.Audio
- **USB** — stack completo: almacenamiento, teclado/ratón físico
- **Filesystem encryption** — Btrfs encryption por bloque (o dm-crypt equivalente)
- **TLS / HTTPS** — sobre Bazzulto.Crypto, wget HTTPS, SSH
- **Contenedores** — namespaces de PID/mount/net
- **Virtualización** — KVM hypervisor
- **Instalador** — instalar Bazzulto desde ISO a disco real
