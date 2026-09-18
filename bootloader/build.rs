use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Copy the (placeholder) memory map into OUT_DIR and put it on the linker
    // search path.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // Linker args: link.x from cortex-m-rt, link-rp.x from embassy-rp (boot2
    // section placement), defmt.x from defmt. --nmagic disables page padding.
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
