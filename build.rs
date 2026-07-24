fn main() {
    // Rust no longer links advapi32 via the standard library (rust-lang/rust#138233).
    // libgit2 still uses Advapi32 APIs (CryptoAPI, registry, security tokens), so the
    // final Windows link of this crate must request it explicitly.
    // See: https://github.com/rust-lang/git2-rs/issues/1142
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
