#!/usr/bin/env bash
# setup-macos.sh — Install all tools required to build and run Bazzulto OS on macOS.
#
# Run once on a fresh Mac:
#   bash scripts/setup-macos.sh
#
# What this installs:
#   - Homebrew (if missing)
#   - QEMU (qemu-system-aarch64 + EDK2 UEFI firmware)
#   - xorriso  (ISO builder, needed for make run-iso)
#   - Rust nightly toolchain via rustup
#   - aarch64-unknown-none target
#   - llvm-tools and rust-src components (needed for bare-metal kernel build)

set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

print_step() { printf '\n\033[1;34m==> %s\033[0m\n' "$1"; }
print_ok()   { printf '\033[1;32m    OK: %s\033[0m\n' "$1"; }
print_skip() { printf '\033[0;33m    SKIP: %s (already installed)\033[0m\n' "$1"; }

# ---------------------------------------------------------------------------
# 1. Homebrew
# ---------------------------------------------------------------------------

print_step "Homebrew"
if command -v brew &>/dev/null; then
    print_skip "brew"
else
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    # Add brew to PATH for the rest of this script (Apple Silicon default path)
    eval "$(/opt/homebrew/bin/brew shellenv)" 2>/dev/null || eval "$(/usr/local/bin/brew shellenv)" 2>/dev/null || true
    print_ok "brew installed"
fi

# ---------------------------------------------------------------------------
# 2. QEMU + EDK2 firmware
# ---------------------------------------------------------------------------

print_step "QEMU"
if brew list qemu &>/dev/null; then
    print_skip "qemu"
else
    brew install qemu
    print_ok "qemu installed"
fi

UEFI_FW="$(brew --prefix qemu)/share/qemu/edk2-aarch64-code.fd"
if [[ -f "$UEFI_FW" ]]; then
    print_ok "EDK2 firmware found at $UEFI_FW"
else
    printf '\033[1;31mERROR: EDK2 firmware not found at %s\033[0m\n' "$UEFI_FW"
    printf 'The Makefile expects it there. Check your QEMU version.\n'
    exit 1
fi

# ---------------------------------------------------------------------------
# 3. xorriso  (ISO builder — needed for make run-iso / make iso)
# ---------------------------------------------------------------------------

print_step "xorriso"
if brew list xorriso &>/dev/null; then
    print_skip "xorriso"
else
    brew install xorriso
    print_ok "xorriso installed"
fi

# ---------------------------------------------------------------------------
# 4. Rust (rustup)
# ---------------------------------------------------------------------------

print_step "Rust / rustup"
if command -v rustup &>/dev/null; then
    print_skip "rustup"
else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
    # Source the env so the rest of this script can use cargo/rustup
    source "$HOME/.cargo/env"
    print_ok "rustup installed"
fi

# Ensure cargo env is loaded even if rustup was already present
source "$HOME/.cargo/env" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 5. Rust nightly toolchain
# ---------------------------------------------------------------------------

print_step "Rust nightly toolchain"
if rustup toolchain list | grep -q 'nightly'; then
    print_skip "nightly toolchain"
else
    rustup toolchain install nightly
    print_ok "nightly toolchain installed"
fi

rustup default nightly
print_ok "nightly set as default"

# ---------------------------------------------------------------------------
# 6. aarch64-unknown-none target
# ---------------------------------------------------------------------------

print_step "aarch64-unknown-none target"
if rustup target list --installed | grep -q 'aarch64-unknown-none'; then
    print_skip "aarch64-unknown-none"
else
    rustup target add aarch64-unknown-none
    print_ok "aarch64-unknown-none added"
fi

# ---------------------------------------------------------------------------
# 7. Rust components: llvm-tools + rust-src
# ---------------------------------------------------------------------------

print_step "llvm-tools component"
if rustup component list --installed | grep -q 'llvm-tools'; then
    print_skip "llvm-tools"
else
    rustup component add llvm-tools
    print_ok "llvm-tools added"
fi

print_step "rust-src component"
if rustup component list --installed | grep -q 'rust-src'; then
    print_skip "rust-src"
else
    rustup component add rust-src
    print_ok "rust-src added"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

printf '\n\033[1;32m========================================\033[0m\n'
printf '\033[1;32m All tools installed. Ready to build.\033[0m\n'
printf '\033[1;32m========================================\033[0m\n\n'
printf 'Quick start:\n'
printf '  make          # build kernel + disk image\n'
printf '  make run      # build and launch QEMU\n\n'
printf 'If you opened a new terminal and cargo is not found:\n'
printf '  source ~/.cargo/env\n\n'
