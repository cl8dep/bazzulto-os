// Build script for bzinit.
//
// Enforces 4 KiB ELF segment alignment by passing a linker script to lld.
// Without this the default aarch64 lld max-page-size (64 KiB) produces
// PT_LOAD segments with p_vaddr % 4096 != 0, which triggers a mapping
// error in the Bazzulto kernel ELF loader.
//
// Runs on the HOST (std is available).

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // bsl.ld lives one directory above bzinit/ (in userspace/bin/).
    let ld_path = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .join("bsl.ld");
    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
    println!("cargo:rerun-if-changed={}", ld_path.display());
}
