//! Local file translation. No account page, remote webview or document upload.
use super::{AppState, lock};
use crate::{
    core::{TranslationRequest, WritingStyle, config::ProviderMode},
    provider::{TranslationProvider, api::ApiProvider},
};
use base64::Engine;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Emitter, Manager, State};

type ActiveJob = Arc<Mutex<Option<(String, Arc<AtomicBool>)>>>;

#[derive(Default)]
pub struct Jobs {
    active: ActiveJob,
    output: Mutex<Option<(String, String, Vec<u8>)>>,
}
// The worker owns the reservation, even if the invoking future is dropped.
struct Reservation {
    active: ActiveJob,
    cancel: Arc<AtomicBool>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if active
            .as_ref()
            .is_some_and(|(_, flag)| Arc::ptr_eq(flag, &self.cancel))
        {
            *active = None;
        }
    }
}
impl Jobs {
    fn reserve(&self, id: String) -> Result<Reservation, String> {
        let mut active = lock(&self.active)?;
        if active.is_some() {
            return Err("Eine Dokumentübersetzung läuft bereits.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some((id, cancel.clone()));
        Ok(Reservation {
            active: self.active.clone(),
            cancel,
        })
    }
}

#[derive(Clone, Serialize)]
pub struct Progress {
    id: String,
    completed: usize,
    characters: usize,
}
#[derive(Serialize)]
pub struct Completed {
    id: String,
    name: String,
    bytes: usize,
    segments: usize,
}

#[tauri::command]
pub fn cancel_document(
    window: tauri::WebviewWindow,
    jobs: State<'_, Jobs>,
    id: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    if let Some((active, cancel)) = &*lock(&jobs.active)?
        && *active == id
    {
        cancel.store(true, Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub async fn translate_document(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: String,
    name: String,
    encoded: String,
    source: String,
    target: String,
) -> Result<Completed, String> {
    if window.label() != "main" || id.len() > 100 || id.is_empty() {
        return Err("Ungültige Dokumentanfrage.".into());
    }
    let request = TranslationRequest {
        text: "validate".into(),
        source: source.clone(),
        target: target.clone(),
        style: WritingStyle::Default,
    };
    request.validate()?;
    if encoded.len() > crate::document::MAX_BYTES.div_ceil(3) * 4 {
        return Err("Maximal 32 MB pro Dokument.".into());
    }
    let name = PathBuf::from(&name)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Ungültiger Dateiname.")?
        .to_owned();
    let extension = PathBuf::from(&name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["pdf", "docx", "pptx"].contains(&extension.as_str()) {
        return Err("Bitte PDF, DOCX oder PPTX auswählen.".into());
    }
    let jobs = app.state::<Jobs>();
    let reservation = jobs.reserve(id.clone())?;
    let cancel = reservation.cancel.clone();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Ungültige Dateidaten.")?;
    if cancel.load(Ordering::Acquire) {
        return Err("Dokumentübersetzung abgebrochen.".into());
    }
    *lock(&jobs.output)? = None;
    let runtime = tokio::runtime::Handle::current();
    let worker_app = app.clone();
    let worker_id = id.clone();
    let output_name = format!(
        "{}_{}.{}",
        PathBuf::from(&name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy(),
        target,
        extension
    );
    let worker_name = output_name.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<Completed> {
        let _reservation = reservation;
        let state=worker_app.state::<AppState>(); let config=lock(&state.config).map_err(anyhow::Error::msg)?.clone();
        let api; let provider:&dyn TranslationProvider=if config.provider==ProviderMode::Web { &state.web } else { api=ApiProvider::new(config.provider)?; &api };
        let mut cache=HashMap::new(); let mut cache_bytes=0usize; let mut completed=0; let mut characters=0;
        let output=crate::document::translate_cancellable(bytes,&extension,|text| {
            anyhow::ensure!(!cancel.load(Ordering::Acquire),"Dokumentübersetzung abgebrochen.");
            characters+=text.chars().count();
            anyhow::ensure!(characters<=300_000 && completed<10_000,"Dokument überschreitet 300.000 Zeichen oder 10.000 Textbereiche.");
            let translated=if let Some(value)=cache.get(text){String::clone(value)}else{
                let request=TranslationRequest{text:text.into(),source:source.clone(),target:target.clone(),style:WritingStyle::Default};
                let result=runtime.block_on(async {
                    tokio::select! {
                        result=provider.translate(&request)=>result.map(|r|r.text).map_err(anyhow::Error::from),
                        _=async {while !cancel.load(Ordering::Acquire){tokio::time::sleep(std::time::Duration::from_millis(50)).await}}=>Err(anyhow::anyhow!("Dokumentübersetzung abgebrochen."))
                    }
                })?;
                anyhow::ensure!(result.len()<=20_000,"Übersetzung ist zu groß.");
                cache_bytes=cache_bytes.checked_add(text.len()+result.len()).ok_or_else(||anyhow::anyhow!("Übersetzungsspeicher ist zu groß."))?;
                anyhow::ensure!(cache_bytes<=16*1024*1024,"Übersetzungsspeicher überschreitet 16 MB.");
                cache.insert(text.to_owned(),result.clone()); result
            };
            completed+=1;
            let _=worker_app.emit_to("main","document-progress",Progress{id:worker_id.clone(),completed,characters});
            Ok(translated)
        }, &mut || { anyhow::ensure!(!cancel.load(Ordering::Acquire),"Dokumentübersetzung abgebrochen."); Ok(()) })?;
        anyhow::ensure!(!cancel.load(Ordering::Acquire),"Dokumentübersetzung abgebrochen.");
        anyhow::ensure!(output.len()<=128*1024*1024,"Ausgabedokument ist zu groß.");
        let bytes=output.len();
        *lock(&worker_app.state::<Jobs>().output).map_err(anyhow::Error::msg)?=Some((worker_id.clone(),worker_name.clone(),output));
        Ok(Completed{id:worker_id,name:worker_name,bytes,segments:completed})
    }).await.map_err(|_|"Dokumentverarbeitung fehlgeschlagen.".to_string()).and_then(|r|r.map_err(|e|format!("{e:#}")))
}

#[tauri::command]
pub async fn save_document(
    window: tauri::Window,
    app: tauri::AppHandle,
    id: String,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    let (name, bytes) = {
        let jobs = app.state::<Jobs>();
        let output = lock(&jobs.output)?;
        let (stored, name, bytes) = output.as_ref().ok_or("Kein fertiges Dokument.")?;
        if *stored != id {
            return Err("Dokument ist nicht mehr aktuell.".into());
        }
        (name.clone(), bytes.clone())
    };
    #[cfg(target_os = "linux")]
    let destination = choose_download_portal(&name).await?;
    #[cfg(not(target_os = "linux"))]
    let destination = {
        // The owned native dialog must run on the application's UI thread.
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let mut destination = PathBuf::from(name);
            let result = if choose_download(&mut destination, &window) {
                Some(destination)
            } else {
                None
            };
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
        let Some(destination) = rx.await.map_err(|e| e.to_string())? else {
            return Ok(false);
        };
        Some(destination)
    };
    let Some(destination) = destination else {
        return Ok(false);
    };
    tokio::task::spawn_blocking(move || {
        let parent = destination.parent().ok_or("Ungültiger Speicherort.")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(&destination).map_err(|e| e.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "windows")]
fn choose_download(destination: &mut std::path::PathBuf, owner: &tauri::Window) -> bool {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    let mut filename = vec![0u16; 32768];
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    for (out, c) in filename.iter_mut().take(32767).zip(name.encode_utf16()) {
        *out = c;
    }
    let filter: Vec<u16> = "Dateien\0*.*\0\0".encode_utf16().collect();
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.hwndOwner = owner
        .hwnd()
        .map(|h| h.0 as _)
        .unwrap_or(std::ptr::null_mut());
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.Flags = OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return false;
    }
    let end = filename
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(filename.len());
    *destination = std::path::PathBuf::from(String::from_utf16_lossy(&filename[..end]));
    destination.is_absolute()
}
#[cfg(target_os = "linux")]
async fn choose_download_portal(name: &str) -> Result<Option<PathBuf>, String> {
    use ashpd::desktop::{ResponseError, file_chooser::SelectedFiles};
    let response = async {
        SelectedFiles::save_file()
            .title("Dokument speichern")
            .current_name(name)
            .modal(true)
            .send()
            .await?
            .response()
    }
    .await;
    match response {
        Ok(files) => {
            let uri = files.uris().first().ok_or("Kein Speicherort erhalten.")?;
            let path = uri
                .to_file_path()
                .map_err(|_| "Ungültiger lokaler Speicherort.")?;
            if !path.is_absolute() {
                return Err("Ungültiger Speicherort.".into());
            }
            Ok(Some(path))
        }
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
        Err(error) => Err(format!(
            "Speicherdialog konnte nicht geöffnet werden: {error}"
        )),
    }
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn choose_download(_destination: &mut std::path::PathBuf, _owner: &tauri::Window) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservation_blocks_decode_overlap_and_releases_on_decode_failure() {
        let jobs = Jobs::default();
        let decode = || -> Result<(), String> {
            let _reservation = jobs.reserve("first".into())?;
            assert!(jobs.reserve("second".into()).is_err());
            base64::engine::general_purpose::STANDARD
                .decode("invalid!")
                .map_err(|_| "decode failed".to_string())?;
            Ok(())
        };
        assert!(decode().is_err());
        assert!(jobs.reserve("third".into()).is_ok());
    }
    #[test]
    fn worker_reservation_survives_cancellation_until_worker_drops() {
        let jobs = Jobs::default();
        let reservation = jobs.reserve("first".into()).unwrap();
        reservation.cancel.store(true, Ordering::Release);
        assert!(jobs.reserve("second".into()).is_err());
        let worker = std::thread::spawn(move || {
            drop(reservation);
        });
        worker.join().unwrap();
        assert!(jobs.reserve("third".into()).is_ok());
    }
}
