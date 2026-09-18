use std::env;
use std::fs;
use std::path::PathBuf;

/// Frozen embassy revision used for both the `[patch.crates-io]` pins and the
/// CYW43 firmware blobs, so the image is reproducible.
const REV: &str = "3cd51e6d8eb6aff8b0d64d9e56a75a538bcfc65a";

/// Proprietary Infineon/CYW43 blobs plus the binary licence. Never committed;
/// fetched here into the gitignored `cyw43-firmware/` directory.
const FILES: &[&str] = &[
    "43439A0.bin",
    "43439A0_btfw.bin",
    "nvram_rp2040.bin",
    "43439A0_clm.bin",
    "LICENSE-permissive-binary-license-1.0.txt",
];

/// Exact byte length of each of the four proprietary blobs, as recorded in the
/// design (and pinned to the frozen `REV`). Asserting them catches a truncated
/// download or a silently substituted file without a checksum framework.
const BLOB_SIZES: &[(&str, u64)] = &[
    ("43439A0.bin", 231_077),
    ("43439A0_btfw.bin", 6_164),
    ("nvram_rp2040.bin", 742),
    ("43439A0_clm.bin", 984),
];

fn main() {
    // 1. memory.x: copy into OUT_DIR and put it on the linker search path.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // 2. Linker args. `link-rp.x` is deliberately absent: it only defines the
    //    `.boot2` section, and the app must not embed boot2 (GL-3 owns it).
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    // 3. Fetch the CYW43 blobs if they are not already present.
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("cyw43-firmware");
    fs::create_dir_all(&dir).unwrap();
    // Re-run when the firmware directory changes (e.g. a blob is deleted or
    // replaced), so cargo does not cache a build against missing blobs.
    println!("cargo:rerun-if-changed={}", dir.display());
    for file in FILES {
        let dest = dir.join(file);
        println!("cargo:rerun-if-changed={}", dest.display());
        if dest.exists() {
            continue;
        }
        let url = format!("https://github.com/embassy-rs/embassy/raw/{REV}/cyw43-firmware/{file}");
        let bytes = fetch(&url).unwrap_or_else(|e| panic!("failed to download {url}: {e}"));
        fs::write(&dest, bytes).unwrap();
    }

    // 4. Integrity guard: pin each blob to its recorded length. A mismatch
    //    (truncated download, wrong file) fails the build with the exact path.
    for (file, expected) in BLOB_SIZES {
        let path = dir.join(file);
        let actual = fs::metadata(&path).unwrap().len();
        assert_eq!(
            actual,
            *expected,
            "{} has {} bytes but the pinned {REV} blob is {expected} bytes",
            path.display(),
            actual,
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url).call().map_err(|e| e.to_string())?;
    response.body_mut().read_to_vec().map_err(|e| e.to_string())
}
