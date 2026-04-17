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
BSL_MANIFEST := userspace/Cargo.toml
KERNEL_DIR   := kernel
KERNEL_ELF   := kernel/target/$(BSL_TARGET)/debug/bazzulto
KERNEL_ELF_R := kernel/target/$(BSL_TARGET)/release/bazzulto

# ---------------------------------------------------------------------------
# Userspace binary paths (release build, aarch64-unknown-none)
# ---------------------------------------------------------------------------

BSL_BIN_DIR := userspace/target/$(BSL_TARGET)/release
BSL_SVC_DIR := userspace/config/services
BSL_FONT_DIR := userspace/assets/fonts

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
	DIR:/system/include \
	DIR:/system/share \
	DIR:/system/config \
	DIR:/system/fonts \
	DIR:/system/fonts/JetBrainsMono \
	DIR:/data \
	DIR:/data/temp \
	DIR:/data/logs \
	DIR:/system/config/policies \
	DIR:/data/test \
	DIR:/dev \
	DIR:/proc \
	DIR:/apps \

# All file mappings for the disk image: host_path:target_path
DISK_FILES := \
	$(_BSL_BIN_MAPPINGS) \
	$(BSL_FONT_DIR)/JetBrainsMono/JetBrainsMono-Regular.ttf:/system/fonts/JetBrainsMono/JetBrainsMono-Regular.ttf \
	userspace/config/disk-mounts:/system/config/disk-mounts \
	userspace/config/system_config/passwd:/system/config/passwd \
	userspace/config/system_config/shadow:/system/config/shadow \
	userspace/config/system_config/group:/system/config/group \
	userspace/config/system_config/hostname:/system/config/hostname \
	userspace/config/test_files/root_secret.txt:/data/test/root_secret.txt \
	userspace/config/test_files/public.txt:/data/test/public.txt \
	userspace/config/test_files/script.sh:/data/test/script.sh \
	userspace/target/aarch64-unknown-none/release/hello:/data/test/hello \
	userspace/target/aarch64-unknown-none/release/readhome:/data/test/readhome \
	tests/libc/test_hello:/data/test/test_hello \
	tests/libc/test_stdlib:/data/test/test_stdlib \
	tests/libc/test_string:/data/test/test_string \
	tests/libc/test_signal:/data/test/test_signal \
	tests/libc/test_time:/data/test/test_time \
	tests/libc/test_pthread:/data/test/test_pthread

# ---------------------------------------------------------------------------
# Default: build everything (kernel + disk image)
# ---------------------------------------------------------------------------

.PHONY: all
all: kernel disk disk2

# ---------------------------------------------------------------------------
# BSL — Rust userspace workspace (bzinit, bzctl, bzsh, bzdisplayd, …)
# ---------------------------------------------------------------------------

.PHONY: bsl
bsl: musl-patch
	$(CARGO) build --release --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST)

.PHONY: bsl-debug
bsl-debug: musl-patch
	$(CARGO) build --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST)

# Build a single crate from the BSL workspace (usage: make bsl-crate CRATE=bzinit)
.PHONY: bsl-crate
bsl-crate: musl-patch
	$(CARGO) build --release --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST) -p $(CRATE)

.PHONY: bsl-crate-debug
bsl-crate-debug: musl-patch
	$(CARGO) build --target $(BSL_TARGET) \
	  --manifest-path $(BSL_MANIFEST) -p $(CRATE)

# ---------------------------------------------------------------------------
# Kernel — independent of BSL (no embedded ELFs; reads from disk at runtime)
# ---------------------------------------------------------------------------

.PHONY: kernel
kernel: musl-patch
	cd $(KERNEL_DIR) && $(CARGO) build

.PHONY: kernel-release
kernel-release: musl-patch
	cd $(KERNEL_DIR) && $(CARGO) build --release

# ---------------------------------------------------------------------------
# musl patches — overlay Bazzulto-specific files onto the musl submodule.
#
# musl is a read-only submodule (upstream git.musl-libc.org).  Files that
# must differ from upstream live in userspace/libraries/libc/patches/ and
# mirror the musl source tree layout.  Run `make musl-patch` once after
# cloning, and again whenever patches/ is updated.
# ---------------------------------------------------------------------------

MUSL_DIR      := userspace/libraries/libc/musl
MUSL_SYSROOT  := userspace/libraries/libc/musl-sysroot
PATCHES_DIR   := userspace/libraries/libc/patches/musl

.PHONY: musl-patch
musl-patch:
	@echo "Applying Bazzulto patches to musl submodule..."
	@find $(PATCHES_DIR) -type f | while read src; do \
	    dst=$(MUSL_DIR)/$${src#$(PATCHES_DIR)/}; \
	    mkdir -p $$(dirname $$dst); \
	    cp $$src $$dst; \
	    echo "  patched: $$dst"; \
	done
	@echo "Done."

# ---------------------------------------------------------------------------
# musl libc — static library for C programs
# ---------------------------------------------------------------------------

.PHONY: musl
musl: musl-patch
	@if [ ! -f $(MUSL_DIR)/config.mak ]; then \
	    echo "Configuring musl..."; \
	    cd $(MUSL_DIR) && ./configure \
	        --target=aarch64-elf \
	        --prefix=$(CURDIR)/$(MUSL_SYSROOT) \
	        --disable-shared \
	        CROSS_COMPILE=aarch64-elf-; \
	fi
	$(MAKE) -C $(MUSL_DIR) -j$$(sysctl -n hw.ncpu 2>/dev/null || echo 4)
	@rm -rf $(MUSL_SYSROOT)
	@mkdir -p $(MUSL_SYSROOT)
	$(MAKE) -C $(MUSL_DIR) install

# ---------------------------------------------------------------------------
# Timezone database — compiled from the IANA source (third_party/tz).
#
# Requires `zic` (available on macOS via Xcode; on Linux via tzdata or libc-bin).
# Compiles POSIX-only zones (no leap-second variants) into third_party/zoneinfo/.
# The resulting binary TZif files are included in disk.img under
# /system/share/zoneinfo/.
# ---------------------------------------------------------------------------

ZIC              := zic
ZONEINFO_SRCDIR  := third_party/tz
ZONEINFO_OUTDIR  := third_party/zoneinfo

# Sentinel file: rebuilt whenever the tz source files change.
ZONEINFO_STAMP   := $(ZONEINFO_OUTDIR)/.built

$(ZONEINFO_STAMP): $(ZONEINFO_SRCDIR)/tzdata.zi
	mkdir -p $(ZONEINFO_OUTDIR)
	$(ZIC) -d $(ZONEINFO_OUTDIR) $(ZONEINFO_SRCDIR)/tzdata.zi
	touch $(ZONEINFO_STAMP)

# tzdata.zi is generated by the tz Makefile from the raw source files.
$(ZONEINFO_SRCDIR)/tzdata.zi: $(ZONEINFO_SRCDIR)/northamerica \
                               $(ZONEINFO_SRCDIR)/europe        \
                               $(ZONEINFO_SRCDIR)/asia          \
                               $(ZONEINFO_SRCDIR)/australasia   \
                               $(ZONEINFO_SRCDIR)/africa        \
                               $(ZONEINFO_SRCDIR)/antarctica    \
                               $(ZONEINFO_SRCDIR)/southamerica  \
                               $(ZONEINFO_SRCDIR)/etcetera      \
                               $(ZONEINFO_SRCDIR)/backward
	$(MAKE) -C $(ZONEINFO_SRCDIR) tzdata.zi

.PHONY: zoneinfo
zoneinfo: $(ZONEINFO_STAMP)

# ---------------------------------------------------------------------------
# Disk image — built with the host Rust toolchain, no external tools needed.
# ---------------------------------------------------------------------------

MKFATIMG     := tools/mkfatimg
MKFATIMG_BIN := $(MKFATIMG)/target/release/mkfatimg

$(MKFATIMG_BIN): $(MKFATIMG)/src/main.rs $(MKFATIMG)/Cargo.toml
	$(CARGO) build --release --manifest-path $(MKFATIMG)/Cargo.toml

MKBTRFS      := tools/mkfs_btrfs
MKBTRFS_BIN  := $(MKBTRFS)/target/release/mkfs_btrfs

$(MKBTRFS_BIN): $(MKBTRFS)/src/main.rs $(MKBTRFS)/Cargo.toml
	$(CARGO) build --release --manifest-path $(MKBTRFS)/Cargo.toml

# Root disk — 1 GiB Btrfs volume containing the entire system.
# Btrfs is the default root filesystem from v1.0 onward.
.PHONY: disk
disk: bsl zoneinfo $(MKBTRFS_BIN)
	$(MKBTRFS_BIN) disk.img 1024 --label BAZZULTO $(DISK_FILES) $(BSL_DIRS) \
	  TREE:$(BSL_SVC_DIR):/system/config/services \
	  TREE:$(ZONEINFO_OUTDIR):/system/share/zoneinfo \
	  $(MUSL_SYSROOT)/lib/libc.a:/system/lib/libc.a \
	  $(MUSL_SYSROOT)/lib/crt1.o:/system/lib/crt1.o \
	  $(MUSL_SYSROOT)/lib/crti.o:/system/lib/crti.o \
	  $(MUSL_SYSROOT)/lib/crtn.o:/system/lib/crtn.o \
	  TREE:$(MUSL_SYSROOT)/include:/system/include

# Second disk — 2 GiB Btrfs volume, mounted at /home/user by the kernel.
.PHONY: disk2
disk2: $(MKBTRFS_BIN)
	$(MKBTRFS_BIN) disk2.img 2048 --label BZZUTHOME DIR:/home DIR:/home/user

# Legacy FAT32 disk targets — retained for ESP and backwards compatibility.
.PHONY: disk-fat32
disk-fat32: bsl zoneinfo $(MKFATIMG_BIN)
	$(MKFATIMG_BIN) disk.img 1024 --volume-id BA270001 $(DISK_FILES) $(BSL_DIRS) \
	  TREE:$(BSL_SVC_DIR):/system/config/services \
	  TREE:$(ZONEINFO_OUTDIR):/system/share/zoneinfo

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
	    -drive file=disk2.img,format=raw,if=none,id=disk1,snapshot=on \
	    -device virtio-blk-device,drive=disk1 \
	    -monitor none \
	    -serial stdio \
	    -parallel none

# The ESP drive is attached after the data disks so it gets a higher virtio-mmio
# slot and does not shift disk.img / disk2.img away from diska / diskb.
# edk2 sees all virtio-blk devices regardless of slot order, so boot still works.
QEMU_BOOT_ESP := -drive file=fat:rw:esp,format=raw,if=none,id=esp \
	    -device virtio-blk-device,drive=esp

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
run: kernel disk disk2
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display cocoa

.PHONY: debug-run
debug-run: kernel disk disk2
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display none

.PHONY: debug-gdb
debug-gdb: kernel disk disk2
	cp $(KERNEL_ELF) esp/bazzulto.elf
	$(QEMU) $(QEMU_COMMON) $(QEMU_BOOT_ESP) -display none -s -S

.PHONY: run-release
run-release: kernel-release disk disk2
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
	$(CARGO) clean --manifest-path $(MKBTRFS)/Cargo.toml
	rm -f esp/bazzulto.elf disk.img disk2.img $(ISO)
