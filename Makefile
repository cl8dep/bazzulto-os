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
    kernel/drivers/console/console.c \
    kernel/memory/physical_memory.c \
    kernel/memory/virtual_memory.c \
    kernel/memory/heap.c

ASM_SOURCES := \
    kernel/arch/arm64/boot/start.S

OBJECTS := $(C_SOURCES:.c=.o) $(ASM_SOURCES:.S=.o)

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
	    -m 256M \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -display cocoa \
	    -monitor none \
	    -serial none \
	    -parallel none &

clean:
	rm -f $(OBJECTS) bazzulto.elf esp/bazzulto.elf
