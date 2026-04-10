# Bazzulto OS build system

# --- Toolchain ---
CC      := aarch64-elf-gcc
AS      := aarch64-elf-gcc   # GCC can assemble .S files directly
LD      := aarch64-elf-ld

# --- Flags ---
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

# --- Sources ---
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
    kernel/arch/arm64/syscall/syscall.c \
    kernel/drivers/uart/uart.c

ASM_SOURCES := \
    kernel/arch/arm64/boot/start.S \
    kernel/arch/arm64/exceptions/exception_vectors.S \
    kernel/scheduler/context_switch.S

USER_SOURCES := user/test_program.S user/echo.S

OBJECTS := $(C_SOURCES:.c=.o) $(ASM_SOURCES:.S=.o) $(USER_SOURCES:.S=.o)

# --- Targets ---
.PHONY: all clean

all: bazzulto.elf

bazzulto.elf: $(OBJECTS)
	$(LD) $(LDFLAGS) -o $@ $^

%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

%.o: %.S
	$(AS) $(ASFLAGS) -c -o $@ $<

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
	rm -f $(OBJECTS) bazzulto.elf esp/bazzulto.elf qemu_debug.log
