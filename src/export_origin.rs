//! Atomic translated-document exports, including conservative Windows origin metadata.
use std::{io, path::Path};

pub fn save(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    save_prepared(destination, bytes, prepare_origin)
}

#[cfg(not(windows))]
fn prepare_origin(_: &tempfile::NamedTempFile) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn zone_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(":Zone.Identifier");
    name.into()
}

#[cfg(windows)]
fn prepare_origin(file: &tempfile::NamedTempFile) -> io::Result<()> {
    use std::{io::Write, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

    let mut flags = 0;
    // Inspect the actual temporary file's volume, including mounted directories.
    // SAFETY: the owned file handle is live; flags is writable; all other
    // optional output pointers are null with zero lengths.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_file().as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::other(format!(
            "Windows-Herkunftsschutz konnte nicht geprüft werden: {}",
            io::Error::last_os_error()
        )));
    }
    require_named_streams(flags)?;
    // The frontend supplies only document bytes and a filename, so source
    // provenance is unknown. Treat every translated export as Internet-zone
    // content; do not fabricate a source URL or copy arbitrary input streams.
    let path = zone_path(file.path());
    let mut stream = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(origin_error)?;
    stream
        .write_all(b"[ZoneTransfer]\r\nZoneId=3\r\n")
        .map_err(origin_error)?;
    stream.sync_all().map_err(origin_error)?;
    drop(stream);
    if std::fs::read(path).map_err(origin_error)? != b"[ZoneTransfer]\r\nZoneId=3\r\n" {
        return Err(io::Error::other(
            "Windows-Herkunftsschutz wurde nicht gespeichert.",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn origin_error(error: io::Error) -> io::Error {
    io::Error::other(format!(
        "Windows-Herkunftsschutz konnte nicht gespeichert werden: {error}"
    ))
}

#[cfg(windows)]
fn require_named_streams(flags: u32) -> io::Result<()> {
    use windows_sys::Win32::System::SystemServices::FILE_NAMED_STREAMS;
    if flags & FILE_NAMED_STREAMS == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Dieser Speicherort unterstützt den Windows-Herkunftsschutz nicht. Bitte ein NTFS-Laufwerk verwenden.",
        ));
    }
    Ok(())
}

fn save_prepared(
    destination: &Path,
    bytes: &[u8],
    prepare: impl FnOnce(&tempfile::NamedTempFile) -> io::Result<()>,
) -> io::Result<()> {
    use std::io::Write;
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("Ungültiger Speicherort."))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    prepare(&file)?;
    file.as_file().sync_all()?;
    // This is a rename of a prepared file in the destination directory, never
    // a cross-volume copy. On Windows tempfile uses MoveFileExW with
    // MOVEFILE_REPLACE_EXISTING and without MOVEFILE_COPY_ALLOWED, so the file
    // and its verified named streams move together. All fallible preparation
    // occurs before replacing the previous destination.
    file.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

/// Standalone verification used by native Windows CI and local artifact checks.
/// Wine or filesystems without named streams deliberately fail this positive
/// check: they cannot establish that a Windows export carries its origin tag.
pub fn self_test() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("translated.docx");
    std::fs::write(&destination, b"old document")?;
    save(&destination, b"translated document")?;
    anyhow::ensure!(
        std::fs::read(&destination)? == b"translated document",
        "Export bytes differ"
    );
    #[cfg(windows)]
    anyhow::ensure!(
        std::fs::read(zone_path(&destination))? == b"[ZoneTransfer]\r\nZoneId=3\r\n",
        "Origin tag did not survive atomic publication"
    );
    let failure = save_prepared(&destination, b"must not replace", |_| {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "origin protection unavailable",
        ))
    });
    anyhow::ensure!(failure.is_err(), "Unprotected export was accepted");
    anyhow::ensure!(
        std::fs::read(&destination)? == b"translated document",
        "Failed export replaced destination"
    );
    anyhow::ensure!(
        std::fs::read_dir(directory.path())?.count() == 1,
        "Temporary export leaked"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_failure_preserves_existing_document() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("translated.docx");
        std::fs::write(&destination, b"original").unwrap();
        let result = save_prepared(&destination, b"replacement", |_| {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no named streams",
            ))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn unsupported_filesystem_flags_prevent_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("translated.docx");
        std::fs::write(&destination, b"original").unwrap();
        let error =
            save_prepared(&destination, b"replacement", |_| require_named_streams(0)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(std::fs::read(destination).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn marks_and_reads_native_windows_stream_before_publication() {
        let file = tempfile::NamedTempFile::new().unwrap();
        prepare_origin(&file).unwrap();
        assert_eq!(
            std::fs::read(zone_path(file.path())).unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
    }

    #[test]
    fn replaces_existing_document_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("translated.docx");
        std::fs::write(&destination, b"old document").unwrap();
        save(&destination, b"translated document").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"translated document");
        #[cfg(windows)]
        assert_eq!(
            std::fs::read(zone_path(&destination)).unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
