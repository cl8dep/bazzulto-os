# Bazzulto OS build system

# --- Toolchain ---
CC      := aarch64-elf-gcc
AS      := aarch64-elf-gcc   # GCC can assemble .S files directly
LD      := aarch64-elf-ld

# --- Kernel flags ---
# -ffreestanding: no standard library, no OS assumed
# -fno-stack-protector: stack canaries require libc support we don't have
# -mgeneral-regs-only: avoid SIMD/FP registers in kernel (they need extra save/restore)
# -Wall -Wextra: catch common mistakes early
CFLAGS  := -std=c11 -ffreestanding -fno-stack-protector -fno-pic \
           -mgeneral-regs-only -Wall -Wextra \
           -I./include

ASFLAGS := -ffreestanding

LDFLAGS := -nostdlib -static \
           -T kernel/arch/arm64/linker.ld

# --- User-space flags ---
USER_CFLAGS  := -std=c11 -ffreestanding -fno-stack-protector -fno-pic \
                -mgeneral-regs-only -Wall -Wextra
USER_LDFLAGS := -nostdlib -static -T userspace/library/linker.ld

# --- Kernel sources ---
C_SOURCES := \
    kernel/arch/arm64/boot/main.c \
    kernel/arch/arm64/exceptions/exceptions.c \
    kernel/drivers/console/console.c \
    kernel/memory/physical_memory.c \
    kernel/memory/virtual_memory.c \
    kernel/memory/heap.c \
    kernel/scheduler/scheduler.c \
    kernel/scheduler/waitqueue.c \
    kernel/arch/arm64/timer.c \
    kernel/arch/arm64/systemcall/systemcall.c \
    kernel/drivers/uart/uart.c \
    kernel/filesystem/ramfs.c \
    kernel/filesystem/virtual_file_system.c \
    kernel/loader/elf_loader.c

ASM_SOURCES := \
    kernel/arch/arm64/boot/start.S \
    kernel/arch/arm64/exceptions/exception_vectors.S \
    kernel/scheduler/context_switch.S

# No more legacy raw user programs — all programs are now ELF binaries.
USER_RAW_SOURCES :=

# --- User-space library ---
USER_LIB_OBJECTS := userspace/library/startup.o userspace/library/systemcall.o

# --- User-space ELF programs ---
# Each program: compile C → link with library → embed via .incbin into kernel.
USER_ELF_PROGRAMS := userspace/hello/hello.elf \
                     userspace/shell/shell.elf \
                     userspace/ls/ls.elf \
                     userspace/help/help.elf \
                     userspace/echo/echo.elf

USER_ELF_EMBEDS := userspace/hello/hello_embed.o \
                   userspace/shell/shell_embed.o \
                   userspace/ls/ls_embed.o \
                   userspace/help/help_embed.o \
                   userspace/echo/echo_embed.o

KERNEL_OBJECTS := $(C_SOURCES:.c=.o) $(ASM_SOURCES:.S=.o) $(USER_RAW_SOURCES:.S=.o)
OBJECTS := $(KERNEL_OBJECTS) $(USER_ELF_EMBEDS)

# --- Targets ---
.PHONY: all clean

all: bazzulto.elf

bazzulto.elf: $(OBJECTS)
	$(LD) $(LDFLAGS) -o $@ $^

# --- Kernel build rules ---
# -MMD -MP: generate .d dependency files alongside each .o.
# The .d files list which headers each .c file includes, so make
# automatically recompiles any .o when a header it depends on changes.
%.o: %.c
	$(CC) $(CFLAGS) -MMD -MP -c -o $@ $<

%.o: %.S
	$(AS) $(ASFLAGS) -c -o $@ $<

# Include all generated dependency files (ignore missing ones on first build).
DEPENDENCY_FILES := $(KERNEL_OBJECTS:.o=.d)
-include $(DEPENDENCY_FILES)

# --- User-space library ---
userspace/library/startup.o: userspace/library/startup.S
	$(AS) $(ASFLAGS) -c -o $@ $<

userspace/library/systemcall.o: userspace/library/systemcall.S
	$(AS) $(ASFLAGS) -c -o $@ $<

# --- User-space programs ---

# hello
userspace/hello/hello.o: userspace/hello/hello.c userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/hello/hello.elf: $(USER_LIB_OBJECTS) userspace/hello/hello.o
	$(LD) $(USER_LDFLAGS) -o $@ $^

userspace/hello/hello_embed.o: userspace/hello/hello_embed.S userspace/hello/hello.elf
	$(AS) $(ASFLAGS) -c -o $@ $<

# shell
userspace/shell/shell.o: userspace/shell/shell.c userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/shell/shell.elf: $(USER_LIB_OBJECTS) userspace/shell/shell.o
	$(LD) $(USER_LDFLAGS) -o $@ $^

userspace/shell/shell_embed.o: userspace/shell/shell_embed.S userspace/shell/shell.elf
	$(AS) $(ASFLAGS) -c -o $@ $<

# ls
userspace/ls/ls.o: userspace/ls/ls.c userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/ls/ls.elf: $(USER_LIB_OBJECTS) userspace/ls/ls.o
	$(LD) $(USER_LDFLAGS) -o $@ $^

userspace/ls/ls_embed.o: userspace/ls/ls_embed.S userspace/ls/ls.elf
	$(AS) $(ASFLAGS) -c -o $@ $<

# help
userspace/help/help.o: userspace/help/help.c userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/help/help.elf: $(USER_LIB_OBJECTS) userspace/help/help.o
	$(LD) $(USER_LDFLAGS) -o $@ $^

userspace/help/help_embed.o: userspace/help/help_embed.S userspace/help/help.elf
	$(AS) $(ASFLAGS) -c -o $@ $<

# echo
userspace/echo/echo.o: userspace/echo/echo.c userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/echo/echo.elf: $(USER_LIB_OBJECTS) userspace/echo/echo.o
	$(LD) $(USER_LDFLAGS) -o $@ $^

userspace/echo/echo_embed.o: userspace/echo/echo_embed.S userspace/echo/echo.elf
	$(AS) $(ASFLAGS) -c -o $@ $<

# --- QEMU targets ---
QEMU        := qemu-system-aarch64
UEFI_FW     := /opt/homebrew/share/qemu/edk2-aarch64-code.fd

run: bazzulto.elf
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -display cocoa \
	    -monitor none \
	    -serial file:uart.log \
	    -parallel none &
	@echo "UART output → uart.log (tail -f uart.log to watch)"

run-serial: bazzulto.elf
	cp bazzulto.elf esp/bazzulto.elf
	@echo "================================================"
	@echo "  UART serial on TCP port 4444"
	@echo "  Connect with:  nc localhost 4444"
	@echo "================================================"
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -display cocoa \
	    -monitor none \
	    -serial tcp::4444,server,nowait \
	    -parallel none &

debug: bazzulto.elf
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -display cocoa \
	    -monitor none \
	    -serial stdio \
	    -parallel none \
	    -d int,cpu_reset -D qemu_debug.log

gdb: bazzulto.elf
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -display cocoa \
	    -monitor none \
	    -serial stdio \
	    -parallel none \
	    -S -s &
	@echo "GDB stub listening on :1234"
	@echo "Connect with: aarch64-elf-gdb -ex 'target remote :1234' -ex 'symbol-file bazzulto.elf'"

clean:
	rm -f $(KERNEL_OBJECTS) $(USER_LIB_OBJECTS) $(USER_ELF_EMBEDS) \
	      userspace/hello/hello.o userspace/hello/hello.elf \
	      userspace/shell/shell.o userspace/shell/shell.elf \
	      userspace/ls/ls.o userspace/ls/ls.elf \
	      userspace/help/help.o userspace/help/help.elf \
	      userspace/echo/echo.o userspace/echo/echo.elf \
	      bazzulto.elf esp/bazzulto.elf qemu_debug.log
