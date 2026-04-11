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
           -I./include -I./include/libc

ASFLAGS := -ffreestanding

LDFLAGS := -nostdlib -static \
           -T kernel/arch/arm64/linker.ld

# --- User-space flags ---
USER_CFLAGS  := -std=c11 -ffreestanding -fno-stack-protector -fno-pic \
                -mgeneral-regs-only -Wall -Wextra \
                -I./userspace/libc
USER_LDFLAGS := -nostdlib -static -T userspace/library/linker.ld

# --- Platform selection ---
PLATFORM ?= qemu_virt
PLATFORM_DIR := kernel/platform/$(PLATFORM)

# --- Kernel sources ---
C_SOURCES := \
    kernel/arch/arm64/boot/main.c \
    kernel/arch/arm64/exceptions/exceptions.c \
    kernel/drivers/console/console.c \
    kernel/drivers/console/font_latin_extended.c \
    kernel/memory/physical_memory.c \
    kernel/memory/virtual_memory.c \
    kernel/memory/heap.c \
    kernel/scheduler/scheduler.c \
    kernel/scheduler/waitqueue.c \
    kernel/scheduler/pid.c \
    kernel/arch/arm64/systemcall/systemcall.c \
    kernel/filesystem/ramfs.c \
    kernel/filesystem/virtual_file_system.c \
    kernel/filesystem/vfs_scheme.c \
    kernel/filesystem/fs_system.c \
    kernel/filesystem/fs_ram.c \
    kernel/filesystem/fs_proc.c \
    kernel/filesystem/filesystem_disk.c \
    kernel/loader/elf_loader.c \
    kernel/drivers/input/input.c \
    kernel/drivers/tty/tty.c \
    kernel/drivers/keyboard/keymap.c \
    kernel/lib/string.c \
    kernel/lib/stdio.c \
    kernel/lib/stdlib.c \
    kernel/lib/utf8.c \
    kernel/arch/arm64/boot/splash.c

# Platform-specific sources (HAL backends)
C_SOURCES += $(wildcard $(PLATFORM_DIR)/*.c)

ASM_SOURCES := \
    kernel/arch/arm64/boot/start.S \
    kernel/arch/arm64/exceptions/exception_vectors.S \
    kernel/scheduler/context_switch.S

# No more legacy raw user programs — all programs are now ELF binaries.
USER_RAW_SOURCES :=

# --- User-space library ---
USER_LIB_OBJECTS := userspace/library/startup.o userspace/library/systemcall.o \
                    userspace/libc/string.o userspace/libc/stdio.o \
                    userspace/libc/stdlib.o userspace/libc/errno.o \
                    userspace/libc/unistd.o

# --- User-space ELF programs ---
# To add a new program: add its name here and create userspace/<name>/<name>.c
# and userspace/<name>/<name>_embed.S — the rules are generated automatically.
USER_PROGRAMS := hello shell ls help echo cat wc grep head hexdump sleep kill cp rm touch tee ps df mount

USER_ELF_PROGRAMS := $(foreach p,$(USER_PROGRAMS),userspace/$(p)/$(p).elf)
USER_ELF_EMBEDS   := $(foreach p,$(USER_PROGRAMS),userspace/$(p)/$(p)_embed.o)

KERNEL_OBJECTS := $(C_SOURCES:.c=.o) $(ASM_SOURCES:.S=.o) $(USER_RAW_SOURCES:.S=.o)
OBJECTS := $(KERNEL_OBJECTS) $(USER_ELF_EMBEDS)

# --- Targets ---
.PHONY: all clean test

all: bazzulto.elf

test:
	./tests/runner.sh

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

# --- User-space libc ---
userspace/libc/string.o: userspace/libc/string.c userspace/libc/string.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/libc/stdio.o: userspace/libc/stdio.c userspace/libc/stdio.h userspace/libc/string.h userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/libc/stdlib.o: userspace/libc/stdlib.c userspace/libc/stdlib.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/libc/errno.o: userspace/libc/errno.c userspace/libc/errno.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

userspace/libc/unistd.o: userspace/libc/unistd.c userspace/libc/unistd.h userspace/libc/errno.h userspace/library/systemcall.h
	$(CC) $(USER_CFLAGS) -c -o $@ $<

# --- User-space library ---
userspace/library/startup.o: userspace/library/startup.S
	$(AS) $(ASFLAGS) -c -o $@ $<

userspace/library/systemcall.o: userspace/library/systemcall.S
	$(AS) $(ASFLAGS) -c -o $@ $<

# --- User-space programs ---
# Rules are generated for every entry in USER_PROGRAMS.
# Adding a new program only requires adding its name to that list.
define userspace_program
userspace/$(1)/$(1).o: userspace/$(1)/$(1).c userspace/library/systemcall.h
	$$(CC) $$(USER_CFLAGS) -c -o $$@ $$<

userspace/$(1)/$(1).elf: $$(USER_LIB_OBJECTS) userspace/$(1)/$(1).o
	$$(LD) $$(USER_LDFLAGS) -o $$@ $$^

userspace/$(1)/$(1)_embed.o: userspace/$(1)/$(1)_embed.S userspace/$(1)/$(1).elf
	$$(AS) $$(ASFLAGS) -c -o $$@ $$<
endef

$(foreach p,$(USER_PROGRAMS),$(eval $(call userspace_program,$(p))))

# --- QEMU targets ---
QEMU        := qemu-system-aarch64
UEFI_FW     := /opt/homebrew/share/qemu/edk2-aarch64-code.fd

run: bazzulto.elf disk.img
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -device virtio-keyboard-device,bus=virtio-mmio-bus.0 \
	    -drive file=disk.img,format=raw,if=none,id=disk0 \
	    -device virtio-blk-device,drive=disk0 \
	    -display cocoa \
	    -monitor none \
	    -serial stdio \
	    -parallel none &

run-serial: bazzulto.elf disk.img
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
	    -drive file=disk.img,format=raw,if=none,id=disk0 \
	    -device virtio-blk-device,drive=disk0 \
	    -display cocoa \
	    -monitor none \
	    -serial tcp::4444,server,nowait \
	    -parallel none &

debug: bazzulto.elf disk.img
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -drive file=disk.img,format=raw,if=none,id=disk0 \
	    -device virtio-blk-device,drive=disk0 \
	    -display cocoa \
	    -monitor none \
	    -serial stdio \
	    -parallel none \
	    -d int,cpu_reset -D qemu_debug.log

gdb: bazzulto.elf disk.img
	cp bazzulto.elf esp/bazzulto.elf
	$(QEMU) \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 1G \
	    -bios $(UEFI_FW) \
	    -drive file=fat:rw:esp,format=raw \
	    -device ramfb \
	    -drive file=disk.img,format=raw,if=none,id=disk0 \
	    -device virtio-blk-device,drive=disk0 \
	    -display cocoa \
	    -monitor none \
	    -serial stdio \
	    -parallel none \
	    -S -s &
	@echo "GDB stub listening on :1234"
	@echo "Connect with: aarch64-elf-gdb -ex 'target remote :1234' -ex 'symbol-file bazzulto.elf'"

# --- Disk image ---
# Always regenerate on every build (never stale).
.PHONY: disk.img

disk.img: FORCE
	python3 create_disk.py

FORCE:

USER_PROGRAM_OBJECTS := $(foreach p,$(USER_PROGRAMS),userspace/$(p)/$(p).o userspace/$(p)/$(p).elf)

clean:
	rm -f $(KERNEL_OBJECTS) $(USER_LIB_OBJECTS) $(USER_ELF_EMBEDS) \
	      $(USER_PROGRAM_OBJECTS) \
	      userspace/libc/string.o userspace/libc/stdio.o userspace/libc/stdlib.o \
	      userspace/libc/errno.o userspace/libc/unistd.o \
	      bazzulto.elf esp/bazzulto.elf qemu_debug.log \
	      disk.img
