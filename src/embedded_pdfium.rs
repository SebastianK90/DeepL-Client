//! Only the compile-time embedded, bounded payload is loaded. No adjacent DLL,
//! PATH lookup, public cache filename or network download participates at runtime.
use anyhow::{Context, Result, ensure};
use pdfium_render::prelude::*;
use std::{
    io::{Read, Write},
    path::Path,
    sync::OnceLock,
};
struct Loaded {
    pdfium: Pdfium,
    _directory: tempfile::TempDir,
}
pub fn library() -> Result<&'static Pdfium> {
    static INSTANCE: OnceLock<std::result::Result<Loaded, String>> = OnceLock::new();
    INSTANCE
        .get_or_init(|| load().map_err(|e| format!("{e:#}")))
        .as_ref()
        .map(|s| &s.pdfium)
        .map_err(|e| anyhow::anyhow!(e.clone()))
}
fn decompress(payload: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    brotli_decompressor::Decompressor::new(payload, 4096)
        .take(limit as u64 + 1)
        .read_to_end(&mut decoded)?;
    ensure!(decoded.len() <= limit, "Embedded payload exceeds limit");
    Ok(decoded)
}
fn load() -> Result<Loaded> {
    let base = directories::ProjectDirs::from("dev", "Local", "DeepLClient")
        .context("Cache-Verzeichnis nicht verfügbar.")?
        .cache_dir()
        .join("pdf-engine");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true).mode(0o700);
        b.create(&base)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&base)?;
    cleanup_old(&base);
    let directory = tempfile::Builder::new()
        .prefix("engine-")
        .tempdir_in(&base)?;
    let payload = decompress(
        include_bytes!(concat!(env!("OUT_DIR"), "/pdfium.br")),
        32 * 1024 * 1024,
    )?;
    ensure!(
        payload.len().to_string() == env!("PDFIUM_RAW_BYTES"),
        "Embedded PDFium is truncated"
    );
    let path = directory
        .path()
        .join(Pdfium::pdfium_platform_library_name());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&payload)?;
    file.sync_all()?;
    drop(file);
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(&path)
            .context("Die integrierte PDF-Engine konnte nicht geladen werden.")?,
    );
    Ok(Loaded {
        pdfium,
        _directory: directory,
    })
}
fn cleanup_old(base: &Path) {
    let Ok(root) = std::fs::canonicalize(base) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("engine-") {
            continue;
        }
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if !meta.is_dir()
            || !meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 86400)
        {
            continue;
        }
        let Ok(path) = std::fs::canonicalize(entry.path()) else {
            continue;
        };
        if path.parent() == Some(root.as_path()) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
pub fn licenses() -> Result<String> {
    Ok(String::from_utf8(decompress(
        include_bytes!(concat!(env!("OUT_DIR"), "/licenses.br")),
        4 * 1024 * 1024,
    )?)?)
}
#[tauri::command]
pub fn third_party_licenses(window: tauri::WebviewWindow) -> std::result::Result<String, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    licenses().map_err(|_| "Lizenzen konnten nicht geladen werden.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_limits_and_notices() {
        assert!(decompress(include_bytes!(concat!(env!("OUT_DIR"), "/pdfium.br")), 1).is_err());
        let notices = licenses().unwrap();
        assert!(notices.contains("PDFium"));
        assert!(notices.contains("shuai.huang"));
    }
}
