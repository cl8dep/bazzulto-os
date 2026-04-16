// Build script — enforces 4 KiB ELF segment alignment via the BSL linker script.
//
// Without this, the default aarch64 lld max-page-size (64 KiB) produces
// PT_LOAD segments with p_vaddr % 4096 != 0, which triggers a mapping
// error in the Bazzulto kernel ELF loader.
//
// bsl.ld lives two directories above the crate root (at userspace/bsl.ld).
// Runs on the HOST (std is available).

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // crate is at userspace/system/permissiond/ → go up two levels to userspace/
    let ld_path = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("bsl.ld");
    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
    println!("cargo:rerun-if-changed={}", ld_path.display());
}
