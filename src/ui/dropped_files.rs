use anyhow::{Result, ensure};
use base64::Engine;
use serde::Serialize;
use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
pub struct DroppedFile {
    name: String,
    mime: &'static str,
    encoded: String,
}

fn read(path: &Path) -> Result<DroppedFile> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Ungültiger Dateiname."))?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (mime, limit) = match extension.as_str() {
        "png" => ("image/png", 8 * 1024 * 1024),
        "jpg" | "jpeg" => ("image/jpeg", 8 * 1024 * 1024),
        "pdf" => ("application/pdf", 32 * 1024 * 1024),
        "docx" => (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            32 * 1024 * 1024,
        ),
        "pptx" => (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            32 * 1024 * 1024,
        ),
        _ => anyhow::bail!("Bitte PNG, JPEG, PDF, DOCX oder PPTX auswählen."),
    };
    ensure!(
        path.symlink_metadata()?.is_file(),
        "Bitte eine reguläre Datei auswählen."
    );
    let file = std::fs::File::open(path)?;
    ensure!(file.metadata()?.len() <= limit, "Die Datei ist zu groß.");
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Die Datei ist zu groß.");
    Ok(DroppedFile {
        name: name.into(),
        mime,
        encoded: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn uri_path(uri: &str) -> Result<std::path::PathBuf> {
    tauri::Url::parse(uri)?
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Bitte eine lokale Datei auswählen."))
}

fn configure_targets(original: Option<gtk::TargetList>) -> gtk::TargetList {
    let targets = original.unwrap_or_else(|| gtk::TargetList::new(&[]));
    let uri = gtk::gdk::Atom::intern("text/uri-list");
    targets.remove(&uri);
    targets.add(&uri, gtk::TargetFlags::OTHER_APP.bits(), 701);
    targets
}

pub fn install(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let app = window.app_handle().clone();
    window.with_webview(move |platform| {
        use gtk::{
            gdk::{Atom, DragAction},
            prelude::*,
        };
        let widget = platform.inner();
        const TARGET: u32 = 701;
        let targets = configure_targets(widget.drag_dest_get_target_list());
        widget.drag_dest_set(
            gtk::DestDefaults::MOTION | gtk::DestDefaults::HIGHLIGHT,
            &[gtk::TargetEntry::new(
                "text/uri-list",
                gtk::TargetFlags::OTHER_APP,
                TARGET,
            )],
            DragAction::COPY,
        );
        widget.drag_dest_set_target_list(Some(&targets));
        // Stop WebKit's motion handler from fetching URI data before release.
        widget.connect_drag_motion(|_, context, _, _, time| {
            if context
                .list_targets()
                .contains(&Atom::intern("text/uri-list"))
            {
                context.drag_status(DragAction::COPY, time);
                true
            } else {
                false
            }
        });
        let pending = std::rc::Rc::new(std::cell::RefCell::new(None));
        let pending_drop = pending.clone();
        // Request data on the actual drop, independently of Wry's enter/leave state.
        widget.connect_drag_drop(move |widget, context, _, _, time| {
            if !context
                .list_targets()
                .contains(&Atom::intern("text/uri-list"))
            {
                return false;
            }
            *pending_drop.borrow_mut() = Some((context.clone(), time));
            widget.drag_get_data(context, &Atom::intern("text/uri-list"), time);
            true
        });
        widget.connect_drag_data_received(move |widget, context, _, _, data, info, time| {
            if info != TARGET {
                return;
            }
            widget.stop_signal_emission_by_name("drag-data-received");
            let expected = pending
                .borrow()
                .as_ref()
                .is_some_and(|(source, requested)| source == context && *requested == time);
            if !expected {
                return;
            }
            pending.borrow_mut().take();
            let paths = data
                .uris()
                .iter()
                .map(|uri| uri_path(uri.as_str()))
                .collect::<Result<Vec<_>>>();
            context.drag_finish(paths.as_ref().is_ok_and(|p| !p.is_empty()), false, time);
            match paths {
                Ok(paths) => process(app.clone(), &paths),
                Err(error) => {
                    let _ = app.emit_to("main", "notice", error.to_string());
                }
            }
        });
    })
}

fn process(app: tauri::AppHandle, paths: &[std::path::PathBuf]) {
    if paths.len() != 1 {
        let _ = app.emit_to("main", "notice", "Bitte genau eine Datei auswählen.");
        return;
    }
    static READING: AtomicBool = AtomicBool::new(false);
    if READING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let path = paths[0].clone();
    tauri::async_runtime::spawn_blocking(move || {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                READING.store(false, Ordering::Release);
            }
        }
        let _guard = Guard;
        match read(&path) {
            Ok(file) => {
                let _ = app.emit_to("main", "native-file-drop", file);
            }
            Err(error) => {
                let _ = app.emit_to("main", "notice", error.to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a graphical GTK session"]
    fn custom_uri_target_keeps_text_and_html_targets() {
        gtk::init().unwrap();
        use gtk::gdk::Atom;
        let original = gtk::TargetList::new(&[
            gtk::TargetEntry::new("text/plain;charset=utf-8", gtk::TargetFlags::empty(), 1),
            gtk::TargetEntry::new("text/html", gtk::TargetFlags::empty(), 2),
            gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::empty(), 3),
        ]);
        let targets = configure_targets(Some(original));
        assert_eq!(
            targets.find(&Atom::intern("text/plain;charset=utf-8")),
            Some(1)
        );
        assert_eq!(targets.find(&Atom::intern("text/html")), Some(2));
        assert_eq!(targets.find(&Atom::intern("text/uri-list")), Some(701));
    }
    #[test]
    fn native_drop_uris_must_be_local_files() {
        assert_eq!(
            uri_path("file:///tmp/Bild%20%C3%A4.png").unwrap(),
            std::path::PathBuf::from("/tmp/Bild ä.png")
        );
        assert!(uri_path("https://example.invalid/file.png").is_err());
        assert!(uri_path("file://remote-host/tmp/x.png").is_err());
        assert!(uri_path("not a URL").is_err());
    }
    #[test]
    fn accepts_supported_types_and_preserves_names() {
        let root = tempfile::tempdir().unwrap();
        for (name, mime) in [
            ("Bild ä.PNG", "image/png"),
            ("photo.jpeg", "image/jpeg"),
            ("report.PDF", "application/pdf"),
            (
                "x.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                "x.pptx",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ),
        ] {
            let path = root.path().join(name);
            std::fs::write(&path, b"fixture").unwrap();
            let file = read(&path).unwrap();
            assert_eq!(file.name, name);
            assert_eq!(file.mime, mime);
            assert_eq!(file.encoded, "Zml4dHVyZQ==");
        }
    }
    #[test]
    fn exact_image_limit_is_accepted_and_symlinks_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("limit.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();
        assert!(read(&path).is_ok());
        let link = root.path().join("linked.png");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read(&link).is_err());
    }
    #[test]
    fn rejects_directories_unsupported_and_oversized_files() {
        let root = tempfile::tempdir().unwrap();
        assert!(read(root.path()).is_err());
        for (name, length) in [
            ("x.txt", 1),
            ("x.png", 8 * 1024 * 1024 + 1),
            ("x.pdf", 32 * 1024 * 1024 + 1),
        ] {
            let path = root.path().join(name);
            std::fs::File::create(&path)
                .unwrap()
                .set_len(length)
                .unwrap();
            assert!(read(&path).is_err(), "{name}");
        }
    }
}
