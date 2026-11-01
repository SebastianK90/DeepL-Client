use std::path::PathBuf;
fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let library = if target == "windows" {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    };
    let root = std::env::var_os("PDFIUM_ASSET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/pdfium"));
    println!("cargo:rerun-if-env-changed=PDFIUM_ASSET_DIR");
    println!("cargo:rerun-if-changed={}", root.display());
    let raw = std::fs::metadata(root.join(library))
        .expect("Run node scripts/setup-pdfium.cjs before building");
    assert!(
        raw.len() < 32 * 1024 * 1024,
        "Unexpected PDFium payload size"
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let compressed = root.join(format!("{library}.br"));
    std::fs::copy(&compressed, out.join("pdfium.br"))
        .expect("Run setup-pdfium.cjs to prepare embedded payload");
    std::fs::copy(root.join("licenses.br"), out.join("licenses.br"))
        .expect("Run setup-pdfium.cjs to prepare notices");
    println!("cargo:rustc-env=PDFIUM_RAW_BYTES={}", raw.len());
    println!(
        "cargo:warning=Embedded PDFium: {} -> {} bytes",
        raw.len(),
        std::fs::metadata(compressed).unwrap().len()
    );
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && std::env::var("CARGO_CFG_TARGET_FEATURE")
            .unwrap_or_default()
            .split(',')
            .any(|feature| feature == "crt-static")
    {
        // Rust's full static CRT setting owns the runtime selection. Tauri's
        // partial-static helper would otherwise force a conflicting dynamic UCRT.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new().static_vc_runtime(false));
    }
    tauri_build::try_build(attributes).expect("Tauri build configuration failed");
}
