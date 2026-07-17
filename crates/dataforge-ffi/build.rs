// =============================================================================
// DataForge FFI — build.rs (C Header Generation)
// =============================================================================
// Automatically generates a C header file `dataforge.h` during `cargo build`
// using cbindgen. The header is placed in the `target/include/` directory.
//
// Prerequisites: `cargo install cbindgen` or add to build-dependencies.
// =============================================================================

fn main() {
    // Only regenerate header when source changes
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::path::PathBuf::from(&crate_dir).join("include");

    // Ensure the output directory exists
    std::fs::create_dir_all(&out_dir).ok();

    let config = match cbindgen::Config::from_file(
        std::path::PathBuf::from(&crate_dir).join("cbindgen.toml"),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cargo:warning=cbindgen config error: {e}. Using defaults.");
            cbindgen::Config::default()
        }
    };

    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(bindings) => {
            bindings.write_to_file(out_dir.join("dataforge.h"));
            println!("cargo:warning=Generated include/dataforge.h");
        }
        Err(e) => {
            eprintln!("cargo:warning=cbindgen generation failed: {e}. Skipping header generation.");
        }
    }
}
