use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Compile the boot entry assembly.
    // start.S declares _start in .text._start; the linker script forces that
    // section to the beginning of .text so it is the kernel's first instruction.
    let start_s = "src/arch/arm64/boot/start.S";
    let start_o = out_dir.join("start.o");

    let status = Command::new("aarch64-elf-gcc")
        .args([
            "-ffreestanding",
            "-c",
            start_s,
            "-o",
            start_o.to_str().unwrap(),
        ])
        .status()
        .expect("failed to invoke aarch64-elf-gcc — is the AArch64 cross-toolchain installed?");

    assert!(status.success(), "start.S compilation failed");

    // Pass start.o to the linker. Cargo appends link-args after Rust objects,
    // but the linker script's KEEP(*(.text._start)) ensures _start comes first
    // regardless of link order.
    println!("cargo:rustc-link-arg={}", start_o.display());

    println!("cargo:rerun-if-changed={}", start_s);
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=build.rs");
}
