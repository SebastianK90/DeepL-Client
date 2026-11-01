//! Local screen selection and OCR; no screenshot is uploaded.
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use tauri::Manager;
static CAPTURING: AtomicBool = AtomicBool::new(false);
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
#[cfg(any(target_os = "linux", test))]
fn bundled_tesseract(
    executable: &std::path::Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let directory = executable.parent()?;
    let binary = directory.join("tesseract");
    let data = directory.parent()?.join("share/deepl-client/tessdata");
    (binary.is_file() && data.is_dir()).then_some((binary, data))
}
fn image_permit(
    gate: std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    gate.try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("Eine Bilderkennung läuft bereits."))
}
// Inspect headers only: compressed-byte limits do not bound decoded pixel memory.
fn check_image_dimensions(bytes: &[u8]) -> Result<()> {
    check_dimensions(
        bytes,
        16_000_000,
        16_384,
        "Das Bild ist zu groß für die Texterkennung (maximal 16 Millionen Pixel).",
    )
}
#[cfg(target_os = "linux")]
fn check_snapshot_dimensions(bytes: &[u8]) -> Result<()> {
    check_dimensions(
        bytes,
        64_000_000,
        30_000,
        "Der Desktop-Schnappschuss ist zu groß (maximal 64 Millionen Pixel).",
    )
}
fn check_dimensions(bytes: &[u8], max_pixels: u64, max_axis: u32, message: &str) -> Result<()> {
    let invalid = || anyhow::anyhow!("Ungültiger PNG- oder JPEG-Bildkopf.");
    let (width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        anyhow::ensure!(
            bytes.len() >= 33 && bytes[8..16] == *b"\0\0\0\rIHDR",
            "Ungültiger PNG-Bildkopf."
        );
        (
            u32::from_be_bytes(bytes[16..20].try_into()?),
            u32::from_be_bytes(bytes[20..24].try_into()?),
        )
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        loop {
            anyhow::ensure!(bytes.get(at) == Some(&0xff), "Ungültiger JPEG-Bildkopf.");
            while bytes.get(at) == Some(&0xff) {
                at += 1;
            }
            let marker = *bytes.get(at).ok_or_else(invalid)?;
            at += 1;
            // No image data or stand-alone markers may precede the frame header.
            anyhow::ensure!(
                !matches!(marker, 0 | 0x01 | 0xd0..=0xda),
                "JPEG-Bildgröße fehlt."
            );
            let length = bytes.get(at..at + 2).ok_or_else(invalid)?;
            let length = u16::from_be_bytes(length.try_into()?) as usize;
            anyhow::ensure!(length >= 2, "Ungültiger JPEG-Bildkopf.");
            let segment = bytes.get(at..at + length).ok_or_else(invalid)?;
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                anyhow::ensure!(
                    segment.len() >= 8
                        && segment.len() == 8 + 3 * segment[7] as usize
                        && segment[7] > 0,
                    "Ungültiger JPEG-Bildkopf."
                );
                break (
                    u16::from_be_bytes([segment[5], segment[6]]) as u32,
                    u16::from_be_bytes([segment[3], segment[4]]) as u32,
                );
            }
            at += length;
        }
    } else {
        anyhow::bail!("Bitte ein PNG- oder JPEG-Bild auswählen.");
    };
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= max_axis
            && height <= max_axis
            && u64::from(width) * u64::from(height) <= max_pixels,
        "{message}"
    );
    Ok(())
}
fn decode_image(encoded: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    anyhow::ensure!(
        encoded.len() <= MAX_IMAGE_BYTES * 4 / 3 + 8,
        "Das Bild ist zu groß (maximal 8 MB)."
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("Ungültiges Bild."))?;
    anyhow::ensure!(
        bytes.len() <= MAX_IMAGE_BYTES,
        "Das Bild ist zu groß (maximal 8 MB)."
    );
    check_image_dimensions(&bytes)?;
    Ok(bytes)
}
fn ocr_permit() -> Result<tokio::sync::OwnedSemaphorePermit> {
    static IMAGE_GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    image_permit(
        IMAGE_GATE
            .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
            .clone(),
    )
}
// Screenshot selection and OCR share the image worker's reservation on every OS.
fn screenshot_permit() -> Result<tokio::sync::OwnedSemaphorePermit> {
    ocr_permit()
}
#[tauri::command]
pub async fn recognize_image(
    window: tauri::WebviewWindow,
    encoded: String,
    language: String,
) -> Result<String, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    if !crate::core::LANGUAGES.contains(&language.as_str()) {
        return Err("Ungültige Sprache.".into());
    }
    let permit = ocr_permit().map_err(|e| e.to_string())?;
    let bytes = decode_image(&encoded).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        // Keep ownership in the worker even if the invoking future is dropped.
        let _permit = permit;
        read_image_text(&bytes, &language).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Bilderkennung fehlgeschlagen.".to_string())?
}
#[cfg(target_os = "windows")]
fn read_image_text(bytes: &[u8], language: &str) -> Result<String> {
    use windows::{
        Graphics::Imaging::BitmapDecoder,
        Media::Ocr::OcrEngine,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    };
    let _apartment = Apartment::new()?;
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?
        .join()
        .map_err(|_| anyhow::anyhow!("Das Bild konnte nicht gelesen werden."))?;
    let max = OcrEngine::MaxImageDimension()?;
    anyhow::ensure!(
        decoder.PixelWidth()? > 0
            && decoder.PixelHeight()? > 0
            && decoder.PixelWidth()? <= max
            && decoder.PixelHeight()? <= max,
        "Das Bild ist zu groß für die Texterkennung."
    );
    let bitmap = decoder
        .GetSoftwareBitmapConvertedAsync(
            windows::Graphics::Imaging::BitmapPixelFormat::Bgra8,
            windows::Graphics::Imaging::BitmapAlphaMode::Premultiplied,
        )?
        .join()?;
    recognize_bitmap(bitmap, language)
}
#[cfg(target_os = "linux")]
fn read_image_text(bytes: &[u8], language: &str) -> Result<String> {
    let lang = match language.split('-').next().unwrap_or(language) {
        "DE" => "deu",
        "FR" => "fra",
        "CS" => "ces",
        "IT" => "ita",
        "JA" => "jpn",
        _ => "eng",
    };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|path| bundled_tesseract(&path));
    let mut command = std::process::Command::new(
        bundled
            .as_ref()
            .map(|(binary, _)| binary.as_os_str())
            .unwrap_or_else(|| std::ffi::OsStr::new("tesseract")),
    );
    if let Some((_, data)) = &bundled {
        anyhow::ensure!(
            data.join(format!("{lang}.traineddata")).is_file(),
            "OCR-Sprachdaten fehlen im AppImage."
        );
        command.arg("--tessdata-dir").arg(data);
    }
    command.args(["stdin", "stdout", "-l", lang]);
    let out = ocr_process::run(
        command,
        bytes,
        std::time::Duration::from_secs(30),
        64 * 1024,
    )?;
    checked_text(&String::from_utf8(out)?)
}
#[cfg(target_os = "linux")]
mod area;
#[cfg(target_os = "linux")]
mod ocr_process;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn read_image_text(_bytes: &[u8], _language: &str) -> Result<String> {
    anyhow::bail!("Bilderkennung wird auf diesem System nicht unterstützt.")
}
#[cfg(target_os = "windows")]
mod native;
#[cfg(any(target_os = "windows", test))]
mod selection;

struct CaptureGuard(tauri::WebviewWindow);
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = self.0.show();
        let _ = self.0.set_focus();
        CAPTURING.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub async fn capture_screenshot(
    window: tauri::WebviewWindow,
    language: String,
    hint: Option<String>,
) -> Result<Option<String>, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    if !crate::core::LANGUAGES.contains(&language.as_str()) {
        return Err("Ungültige Sprache.".into());
    }
    if hint.as_ref().is_some_and(|hint| hint.chars().count() > 200) {
        return Err("Ungültiger Ziehbereich.".into());
    }
    if CAPTURING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("Eine Screenshot-Erfassung läuft bereits.".into());
    }
    let _guard = CaptureGuard(window.clone());
    let permit = screenshot_permit().map_err(|e| e.to_string())?;
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (language, hint);
        Err("Screenshot-Erfassung ist derzeit nur unter Windows verfügbar.".into())
    }
    #[cfg(target_os = "linux")]
    {
        window.hide().map_err(|e| e.to_string())?;
        tokio::time::sleep(std::time::Duration::from_millis(180)).await;
        let Some(bytes) = portal::snapshot().await.map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let hint = hint.unwrap_or_else(|| "Bereich ziehen · Esc zum Abbrechen".into());
        let Some(bytes) = area::select(window.app_handle().clone(), bytes, hint)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            read_image_text(&bytes, &language)
                .map(Some)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(target_os = "windows")]
    {
        window
            .hide()
            .map_err(|_| "Fenster konnte nicht ausgeblendet werden.")?;
        tokio::time::sleep(std::time::Duration::from_millis(180)).await;
        tokio::task::spawn_blocking(move || {
            // The native worker retains the reservation if its caller is cancelled.
            let _permit = permit;
            windows_capture(
                &language,
                hint.as_deref()
                    .unwrap_or("Bereich ziehen · Esc zum Abbrechen"),
            )
        })
        .await
        .map_err(|_| "Screenshot-Erfassung fehlgeschlagen.".to_string())?
        .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_capture(language: &str, hint: &str) -> Result<Option<String>> {
    anyhow::ensure!(hint.chars().count() <= 200, "Ungültiger Ziehbereich.");
    let Some(image) = native::select(hint)? else {
        return Ok(None);
    };
    let _apartment = Apartment::new()?;
    recognize(image, language).map(Some)
}
#[cfg(target_os = "windows")]
struct Apartment;
#[cfg(target_os = "windows")]
impl Apartment {
    fn new() -> Result<Self> {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|_| {
            anyhow::anyhow!("Windows-Texterkennung konnte nicht initialisiert werden.")
        })?;
        Ok(Self)
    }
}
#[cfg(target_os = "windows")]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}

#[cfg(target_os = "windows")]
fn recognize(image: arboard::ImageData<'_>, language: &str) -> Result<String> {
    use windows::{
        Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
    };
    let max = OcrEngine::MaxImageDimension()? as usize;
    anyhow::ensure!(
        image.width > 0
            && image.height > 0
            && image.width <= max
            && image.height <= max
            && image
                .width
                .checked_mul(image.height)
                .and_then(|p| p.checked_mul(4))
                == Some(image.bytes.len()),
        "Der Screenshot ist zu groß oder ungültig. Bitte einen kleineren Bereich auswählen."
    );
    let mut bgra = image.bytes.into_owned();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        image.width as i32,
        image.height as i32,
    )?;
    recognize_bitmap(bitmap, language)
}

#[cfg(target_os = "windows")]
fn recognize_bitmap(
    bitmap: windows::Graphics::Imaging::SoftwareBitmap,
    language: &str,
) -> Result<String> {
    use anyhow::Context;
    use windows::{Globalization::Language, Media::Ocr::OcrEngine};
    let engine = ocr_language(language).and_then(|code|Language::CreateLanguage(&code.into()).ok()).and_then(|lang|OcrEngine::TryCreateFromLanguage(&lang).ok()).or_else(||OcrEngine::TryCreateFromUserProfileLanguages().ok()).context("Keine Windows-OCR-Sprache installiert. Bitte ein Sprachpaket mit Texterkennung in Windows installieren.")?;
    let recognized = engine
        .RecognizeAsync(&bitmap)
        .context("Windows-Texterkennung fehlgeschlagen.")?
        .join()
        .context("Windows-Texterkennung fehlgeschlagen.")?;
    let mut lines = Vec::new();
    for line in recognized.Lines()? {
        lines.push(line.Text()?.to_string());
    }
    checked_text(&lines.join("\n"))
}

#[cfg(target_os = "windows")]
fn ocr_language(code: &str) -> Option<String> {
    if code == "AUTO" {
        return None;
    }
    Some(
        match code {
            "EN" => "en-US",
            "DE" => "de-DE",
            "CS" | "CZ" => "cs-CZ",
            "JA" => "ja-JP",
            "ZH" | "ZH-HANS" => "zh-CN",
            "FR" => "fr-FR",
            "IT" => "it-IT",
            "ES" => "es-ES",
            "PT" => "pt-PT",
            _ => code,
        }
        .into(),
    )
}

#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn checked_text(text: &str) -> Result<String> {
    let text = text.replace("\r\n", "\n").trim().to_string();
    anyhow::ensure!(!text.is_empty(), "Im Screenshot wurde kein Text erkannt.");
    anyhow::ensure!(
        text.chars().count() <= 5000,
        "Der Screenshot enthält mehr als 5.000 Zeichen."
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_and_image_ocr_share_one_reservation_and_release_after_cancel() {
        let permit = ocr_permit().unwrap();
        assert!(
            screenshot_permit().is_err(),
            "image OCR must block screenshot OCR"
        );
        drop(permit);
        let screenshot = screenshot_permit().unwrap();
        assert!(ocr_permit().is_err(), "screenshot OCR must block image OCR");
        // Cancelling the selector, failing native capture, or completing OCR all
        // drop this reservation before another image worker can proceed.
        drop(screenshot);
        assert!(ocr_permit().is_ok());
    }
    #[test]
    fn appimage_ocr_paths_are_relative_to_the_running_binary() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("usr/bin");
        let data = root.path().join("usr/share/deepl-client/tessdata");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(bin.join("tesseract"), b"fixture").unwrap();
        let (engine, models) = bundled_tesseract(&bin.join("deepl-client")).unwrap();
        assert_eq!(engine, bin.join("tesseract"));
        assert_eq!(models, data);
        assert!(bundled_tesseract(&root.path().join("elsewhere/deepl-client")).is_none());
    }
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        data.extend(width.to_be_bytes());
        data.extend(height.to_be_bytes());
        data.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        data
    }
    #[test]
    fn image_headers_bound_pixels_before_native_decode() {
        assert!(check_image_dimensions(&png(4000, 4000)).is_ok());
        assert!(check_image_dimensions(&png(4001, 4000)).is_err());
        assert!(check_image_dimensions(&png(0, 10)).is_err());
        assert!(check_image_dimensions(&png(16_385, 1)).is_err());
        assert!(check_image_dimensions(&png(u32::MAX, u32::MAX)).is_err());
        assert!(check_image_dimensions(b"\x89PNG\r\n\x1a\nfixture").is_err());
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 1, 1, 0x11, 0,
        ];
        assert!(check_image_dimensions(&jpeg).is_ok());
        for end in 0..jpeg.len() {
            assert!(check_image_dimensions(&jpeg[..end]).is_err());
        }
        let mut oversized = jpeg;
        oversized[13..17].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        assert!(check_image_dimensions(&oversized).is_err());
        let mut invalid_segment = jpeg;
        invalid_segment[4..6].copy_from_slice(&[0, 1]);
        assert!(check_image_dimensions(&invalid_segment).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_snapshot_can_exceed_ocr_crop_budget_but_stays_bounded() {
        let dual_4k = png(7680, 2160);
        assert!(check_snapshot_dimensions(&dual_4k).is_ok());
        assert!(check_image_dimensions(&dual_4k).is_err());
        assert!(check_snapshot_dimensions(&png(16000, 8000)).is_err());
    }
    #[tokio::test]
    async fn image_permit_rejects_overlap_and_releases_on_error() {
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let permit = image_permit(gate.clone()).unwrap();
        assert!(image_permit(gate.clone()).is_err());
        let error = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            decode_image("invalid base64")
        })
        .await
        .unwrap();
        assert!(error.is_err());
        assert!(image_permit(gate).is_ok());
    }
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "Windows image decoder + local OCR on a generated synthetic PNG"]
    fn live_image_ocr() {
        let bytes = std::fs::read(".tools/image-ocr-fixture.png").unwrap();
        let text = read_image_text(&bytes, "EN").unwrap();
        assert!(text.contains("Hello"), "{text}");
        std::fs::write("docs/evidence/image-ocr-live.txt", text).unwrap();
    }
    #[test]
    fn image_input_rejects_nonimages_and_oversize_before_native_decode() {
        use base64::Engine;
        assert!(decode_image("file:///private.png").is_err());
        assert!(
            decode_image(&base64::engine::general_purpose::STANDARD.encode(b"<svg/>")).is_err()
        );
        assert!(decode_image(&"A".repeat(MAX_IMAGE_BYTES * 4 / 3 + 10)).is_err());
        assert!(
            decode_image(&base64::engine::general_purpose::STANDARD.encode(png(100, 100))).is_ok()
        );
    }
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "interactive native selector; select the synthetic screenshot fixture"]
    fn live_native_selector() {
        use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
        let sequence = unsafe { GetClipboardSequenceNumber() };
        let image = native::select("Drag the test text · Esc to cancel")
            .unwrap()
            .expect("selection required");
        assert_eq!(
            sequence,
            unsafe { GetClipboardSequenceNumber() },
            "capture must not change clipboard"
        );
        let _apartment = Apartment::new().unwrap();
        let text = recognize(image, "EN").unwrap();
        assert!(text.contains("Screenshot test"), "{text}");
        std::fs::write("docs/evidence/native-private-capture.json",serde_json::to_vec_pretty(&serde_json::json!({"date":"2026-10-29","capture":"in-process Win32 GDI and own selector","ocr":"Windows.Media.Ocr","output":text,"clipboard_sequence_unchanged":true,"snipping_tool_invoked":false,"screenshot_file_written":false,"result":"passed"})).unwrap()).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "local Windows OCR on a generated synthetic bitmap fixture"]
    fn live_local_ocr() {
        let _apartment = Apartment::new().unwrap();
        let mut pixels = std::fs::read(".tools/capture-fixture.bgra").unwrap();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let text = recognize(
            arboard::ImageData {
                width: 1000,
                height: 180,
                bytes: pixels.into(),
            },
            "EN",
        )
        .unwrap();
        assert!(text.contains("Screenshot test"), "{text}");
        std::fs::write("docs/evidence/local-ocr-smoke.json",serde_json::to_vec_pretty(&serde_json::json!({"date":"2026-10-29","engine":"Windows.Media.Ocr","input":"Screenshot test 123 / Hello world","output":text,"network_ocr":false,"result":"passed"})).unwrap()).unwrap();
    }
    #[test]
    fn ocr_text_is_unicode_bounded_without_truncation() {
        assert_eq!(checked_text("  Grüße\r\n世界  ").unwrap(), "Grüße\n世界");
        assert!(checked_text(" \r\n ").is_err());
        assert!(checked_text(&"界".repeat(5000)).is_ok());
        assert!(checked_text(&"界".repeat(5001)).is_err());
    }
}

#[derive(serde::Serialize)]
pub(crate) struct CaptureFrame {
    encoded: String,
    width: u32,
    height: u32,
    token: String,
    hint: String,
}
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct CaptureRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[tauri::command]
pub fn capture_area_state(window: tauri::WebviewWindow) -> Result<CaptureFrame, String> {
    if window.label() != "capture-select" {
        return Err("Ungültiges Fenster.".into());
    }
    #[cfg(target_os = "linux")]
    {
        area::state().map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("Aufnahme nicht verfügbar.".into())
    }
}
#[tauri::command]
pub fn capture_area_finish(
    window: tauri::WebviewWindow,
    token: String,
    region: Option<CaptureRect>,
) -> Result<(), String> {
    if window.label() != "capture-select" {
        return Err("Ungültiges Fenster.".into());
    }
    #[cfg(target_os = "linux")]
    {
        area::finish(&token, region).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (token, region);
        Err("Aufnahme nicht verfügbar.".into())
    }
}
