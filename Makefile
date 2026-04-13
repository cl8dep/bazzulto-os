# Bazzulto OS — top-level build system

# ---------------------------------------------------------------------------
# Toolchain
# ---------------------------------------------------------------------------

CARGO        := ~/.cargo/bin/cargo
QEMU         := qemu-system-aarch64
XORRISO      := xorriso
UEFI_FW      := /opt/homebrew/share/qemu/edk2-aarch64-code.fd
ISO          := bazzulto.iso

# ---------------------------------------------------------------------------
# Targets (BSL = Rust userspace workspace)
# ---------------------------------------------------------------------------

BSL_TARGET   := aarch64-unknown-none
BSL_MANIFEST := userspace/bin/Cargo.toml
KERNEL_DIR   := kernel
KERNEL_ELF   := kernel/target/$(BSL_TARGET)/debug/bazzulto
KERNEL_ELF_R := kernel/target/$(BSL_TARGET)/release/bazzulto

# ---------------------------------------------------------------------------
# Userspace binary paths (release build, aarch64-unknown-none)
# ---------------------------------------------------------------------------

BSL_BIN_DIR := userspace/bin/target/$(BSL_TARGET)/release
BSL_SVC_DIR := userspace/services
BSL_FONT_DIR := userspace/fonts

# ---------------------------------------------------------------------------
# Auto-discover all userspace binaries — any executable file in BSL_BIN_DIR
# that is not a .d file and has no extension is a binary to install.
#
# Exclusions: fontmanager (internal tool, not a shell command).
# ---------------------------------------------------------------------------
BSL_BIN_EXCLUDE := fontmanager echo pwd

_BSL_ALL_BINS := $(filter-out \
    $(addprefix $(BSL_BIN_DIR)/,$(BSL_BIN_EXCLUDE)), \
    $(shell find $(BSL_BIN_DIR) -maxdepth 1 -type f ! -name '*.*' 2>/dev/null))

# Convert each host path to host_path:/system/bin/<name> mapping.
_BSL_BIN_MAPPINGS := $(foreach bin,$(_BSL_ALL_BINS),$(bin):/system/bin/$(notdir $(bin)))

# Directories to create on the disk image (no files needed, just the directory).
# mkfatimg creates them via "DIR:target_path" mappings.
BSL_DIRS := \
	DIR:/home \
	DIR:/home/user \
	DIR:/home/user/.bin \
	DIR:/home/user/.lib \
	DIR:/system/lib \
	DIR:/system/share \
	DIR:/data \
	DIR:/data/temp \
	DIR:/data/logs \
	DIR:/dev \
	DIR:/proc \
	DIR:/apps \

# All file mappings for the disk image: host_path:target_path
DISK_FILES := \
	$(_BSL_BIN_MAPPINGS) \
	$(BSL_SVC_DIR)/bzdisplayd.service:/config/bazzulto/services/bzdisplayd.service \
	$(BSL_SVC_DIR)/shell.service:/config/bazzulto/services/shell.service \
	$(BSL_FONT_DIR)/JetBrainsMono/JetBrainsMono-Regular.ttf:/system/fonts/JetBrainsMono/JetBrainsMono-Regular.ttf

# ---------------------------------------------------------------------------
# Default: build everything (kernel + disk image)
# ---------------------------------------------------------------------------

.PHONY: all
all: kernel disk

# ---------------------------------------------------------------------------
# BSL — Rust userspace workspace (bzinit, bzctl, bzsh, bzdisplayd, …)
# ---------------------------------------------------------------------------

.PHONY: bsl
bsl:
	$(CARGO) build --release --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST)

.PHONY: bsl-debug
bsl-debug:
	$(CARGO) build --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST)

# Build a single crate from the BSL workspace (usage: make bsl-crate CRATE=bzinit)
.PHONY: bsl-crate
bsl-crate:
	$(CARGO) build --release --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST) -p $(CRATE)

.PHONY: bsl-crate-debug
bsl-crate-debug:
	$(CARGO) build --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST) -p $(CRATE)

# ---------------------------------------------------------------------------
# Kernel — independent of BSL (no embedded ELFs; reads from disk at runtime)
# ---------------------------------------------------------------------------

.PHONY: kernel
kernel:
	cd $(KERNEL_DIR) && $(CARGO) build

.PHONY: kernel-release
kernel-release:
	cd $(KERNEL_DIR) && $(CARGO) build --release

# ---------------------------------------------------------------------------
# Disk image — built with the host Rust toolchain, no external tools needed.
# ---------------------------------------------------------------------------

MKFATIMG     := tools/mkfatimg
MKFATIMG_BIN := $(MKFATIMG)/target/release/mkfatimg

$(MKFATIMG_BIN): $(MKFATIMG)/src/main.rs $(MKFATIMG)/Cargo.toml
	$(CARGO) build --release --manifest-path $(MKFATIMG)/Cargo.toml

.PHONY: disk
disk: bsl $(MKFATIMG_BIN)
	$(MKFATIMG_BIN) disk.img 256 $(DISK_FILES) $(BSL_DIRS)

# ---------------------------------------------------------------------------
# ISO image — UEFI bootable via xorriso (cross-platform: Linux/macOS/Windows)
#
# Structure inside the ISO mirrors esp/:
#   EFI/BOOT/BOOTAA64.EFI   ← Limine UEFI binary
#   limine.conf
#   bazzulto.elf
#   boot-wallpaper.jpg
#
# El Torito UEFI boot: no emulation, EFI/BOOT/BOOTAA64.EFI is the boot image.
# No BIOS/legacy boot — Bazzulto targets UEFI-only hardware and QEMU virt.
# ---------------------------------------------------------------------------

.PHONY: iso
iso: kernel
	cp $(KERNEL_ELF) esp/bazzulto.elf
	# startup.nsh: UEFI Shell auto-executes this when El Torito auto-boot
	# does not trigger.  Placed temporarily in esp/ for xorriso, then removed.
	printf '\EFI\BOOT\BOOTAA64.EFI\n' > esp/startup.nsh
	$(XORRISO) -as mkisofs \
	    -V 'BAZZULTO' \
	    --efi-boot EFI/BOOT/BOOTAA64.EFI \
	    -no-emul-boot \
	    -efi-boot-part \
	    --efi-boot-image \
	    -o $(ISO) \
	    esp/ ; \
	rm -f esp/startup.nsh

# ---------------------------------------------------------------------------
# Run in QEMU
# ---------------------------------------------------------------------------

# Shared QEMU flags — peripherals common to all run targets.
QEMU_COMMON := \
	    -machine virt \
	    -cpu cortex-a72 \
	    -m 2G \
	    -bios $(UEFI_FW) \
	    -device ramfb \
	    -device virtio-keyboard-device,bus=virtio-mmio-bus.0 \
	    -drive file=disk.img,format=raw,if=none,id=disk0,snapshot=on \
	    -device virtio-blk-device,drive=disk0 \
	    -monitor none \
	    -serial stdio \
	    -parallel none

QEMU_BOOT_ESP := -drive file=fat:rw:esp,format=raw

# ISO boot via virtio-scsi (for run-iso / distribution testing).
# edk2 on -machine virt mounts the El Torito FAT as FS0 but does not
# auto-execute \EFI\BOOT\BOOTAA64.EFI from it reliably — use run-iso
# only when testing the distributable ISO, not for daily development.
QEMU_BOOT_ISO := \
	    -device virtio-scsi-device,id=scsi0 \
	    -drive file=$(ISO),format=raw,if=none,media=cdrom,id=cdrom0 \
	    -device scsi-cd,bus=scsi0.0,drive=cdrom0

# ---------------------------------------------------------------------------
# Daily development targets — boot from esp/ (no xorriso, instant boot).
# ---------------------------------------------------------------------------

.PHONY: run
run: kernel disk
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display cocoa

.PHONY: debug-run
debug-run: kernel disk
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display none

.PHONY: debug-gdb
debug-gdb: kernel disk
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display none -s -S

.PHONY: run-release
run-release: kernel-release disk
	cp $(KERNEL_ELF_R) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display cocoa

# ---------------------------------------------------------------------------
# ISO distribution targets — build ISO and boot from it.
# ---------------------------------------------------------------------------

.PHONY: run-iso
run-iso: iso disk
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ISO) -display cocoa

.PHONY: debug-run-iso
debug-run-iso: iso disk
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ISO) -display none

.PHONY: iso-release
iso-release: kernel-release
	cp $(KERNEL_ELF_R) esp/bazzulto.elf
	printf '\EFI\BOOT\BOOTAA64.EFI\n' > esp/startup.nsh
	$(XORRISO) -as mkisofs \
	    -V 'BAZZULTO' \
	    --efi-boot EFI/BOOT/BOOTAA64.EFI \
	    -no-emul-boot \
	    -efi-boot-part \
	    --efi-boot-image \
	    -o $(ISO) \
	    esp/ ; \
	rm -f esp/startup.nsh

# ---------------------------------------------------------------------------
# Clean
# ---------------------------------------------------------------------------

.PHONY: clean
clean:
	cd $(KERNEL_DIR) && $(CARGO) clean
	$(CARGO) clean --manifest-path $(BSL_MANIFEST)
	$(CARGO) clean --manifest-path $(MKFATIMG)/Cargo.toml
	rm -f esp/bazzulto.elf disk.img $(ISO)
