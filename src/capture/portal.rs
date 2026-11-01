use anyhow::{Result, ensure};
use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime},
};

struct PortalImage(PathBuf);
impl Drop for PortalImage {
    fn drop(&mut self) {
        // Only a fresh regular file returned by the trusted desktop portal is owned here.
        let _ = std::fs::remove_file(&self.0);
    }
}
fn consume(uri: &str, requested: SystemTime) -> Result<Vec<u8>> {
    let uri = tauri::Url::parse(uri)?;
    let path = uri.to_file_path().map_err(|_| {
        anyhow::anyhow!("Das Screenshot-Portal hat keine lokale Bilddatei geliefert.")
    })?;
    let metadata = path.symlink_metadata()?;
    ensure!(metadata.is_file(), "Ungültige Screenshot-Datei.");
    ensure!(
        metadata.modified()?
            >= requested
                .checked_sub(Duration::from_secs(2))
                .unwrap_or(SystemTime::UNIX_EPOCH),
        "Das Screenshot-Portal lieferte eine alte Datei."
    );
    let image = PortalImage(path);
    let file = std::fs::File::open(&image.0)?;
    ensure!(
        file.metadata()?.len() <= super::MAX_SNAPSHOT_BYTES as u64,
        "Das Bild ist zu groß (maximal 32 MB)."
    );
    let mut bytes = Vec::new();
    file.take(super::MAX_SNAPSHOT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= super::MAX_SNAPSHOT_BYTES,
        "Das Bild ist zu groß (maximal 32 MB)."
    );
    super::check_snapshot_dimensions(&bytes)?;
    Ok(bytes)
}

pub(super) async fn snapshot() -> Result<Option<Vec<u8>>> {
    use ashpd::{
        Error,
        desktop::{ResponseError, screenshot::Screenshot},
    };
    let requested = SystemTime::now();
    let response = Screenshot::request()
        .interactive(false)
        .modal(true)
        .send()
        .await;
    let response = match response.and_then(|request| request.response()) {
        Ok(response) => response,
        Err(Error::Response(ResponseError::Cancelled)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let uri = response.uri().to_string();
    tokio::task::spawn_blocking(move || consume(&uri, requested).map(Some)).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_local_portal_image_and_removes_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Screenshot ä.png");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII=").unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let uri = tauri::Url::from_file_path(&path).unwrap();
        assert_eq!(
            consume(uri.as_str(), std::time::SystemTime::now()).unwrap(),
            bytes
        );
        assert!(!path.exists());
    }
    #[test]
    fn failed_images_are_cleaned_but_old_files_and_remote_uris_are_not_consumed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Screenshot.png");
        std::fs::write(&path, b"invalid").unwrap();
        let uri = tauri::Url::from_file_path(&path).unwrap();
        assert!(consume(uri.as_str(), std::time::SystemTime::now()).is_err());
        assert!(!path.exists());
        std::fs::write(&path, b"original").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        assert!(consume(uri.as_str(), std::time::SystemTime::now()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(
            consume(
                "https://example.invalid/picture.png",
                std::time::SystemTime::now()
            )
            .is_err()
        );
    }
    #[test]
    fn refuses_symlinks_without_deleting_target() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("original.png");
        let link = root.path().join("Screenshot.png");
        std::fs::write(&path, b"original").unwrap();
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let uri = tauri::Url::from_file_path(&link).unwrap();
        assert!(consume(uri.as_str(), SystemTime::now()).is_err());
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn oversized_images_are_rejected_and_cleaned() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Screenshot.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        let uri = tauri::Url::from_file_path(&path).unwrap();
        assert!(consume(uri.as_str(), std::time::SystemTime::now()).is_err());
        assert!(!path.exists());
    }
}
