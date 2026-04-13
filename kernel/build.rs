use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Compile the boot entry assembly.
    // start.S declares _start in .text._start; the linker script forces that
    // section to the beginning of .text so it is the kernel's first instruction.
    // Helper: compile one .S file to .o and link it into the kernel.
    let compile_asm = |src: &str, obj_name: &str| {
        let obj = out_dir.join(obj_name);
        let status = Command::new("aarch64-elf-gcc")
            .args(["-ffreestanding", "-c", src, "-o", obj.to_str().unwrap()])
            .status()
            .unwrap_or_else(|_| panic!("failed to invoke aarch64-elf-gcc for {src}"));
        assert!(status.success(), "{src} compilation failed");
        println!("cargo:rustc-link-arg={}", obj.display());
        println!("cargo:rerun-if-changed={src}");
    };

    compile_asm("src/arch/arm64/boot/start.S",             "start.o");
    compile_asm("src/arch/arm64/exceptions/vectors.S",     "vectors.o");
    compile_asm("src/process/context_switch.S",            "context_switch.o");

    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=build.rs");

    // Rebuild kernel when embedded userspace ELFs change.
    let bsl_release = "../userspace/bin/target/aarch64-unknown-none/release";
    println!("cargo:rerun-if-changed={bsl_release}/bzinit");
    println!("cargo:rerun-if-changed={bsl_release}/bzctl");
    println!("cargo:rerun-if-changed={bsl_release}/bzsh");
    println!("cargo:rerun-if-changed=../userspace/services/shell.service");
}
