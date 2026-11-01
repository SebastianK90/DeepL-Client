use super::{CaptureFrame, CaptureRect};
use anyhow::{Result, ensure};
use base64::Engine;
use std::sync::Mutex;
use tauri::Manager;

struct Session {
    token: String,
    frame: Option<CaptureFrame>,
    sender: tokio::sync::oneshot::Sender<Option<CaptureRect>>,
}
static SESSION: Mutex<Option<Session>> = Mutex::new(None);

pub(super) fn state() -> Result<CaptureFrame> {
    SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("Aufnahme nicht verfügbar."))?
        .as_mut()
        .and_then(|s| s.frame.take())
        .ok_or_else(|| anyhow::anyhow!("Aufnahme nicht verfügbar."))
}
pub(super) fn finish(token: &str, region: Option<CaptureRect>) -> Result<()> {
    let mut session = SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("Aufnahme nicht verfügbar."))?;
    ensure!(
        session.as_ref().is_some_and(|s| s.token == token),
        "Aufnahme nicht mehr aktuell."
    );
    if let Some(session) = session.take() {
        let _ = session.sender.send(region);
    }
    Ok(())
}
fn cancel() {
    if let Ok(mut session) = SESSION.lock()
        && let Some(session) = session.take()
    {
        let _ = session.sender.send(None);
    }
}
struct Guard(tauri::AppHandle);
impl Drop for Guard {
    fn drop(&mut self) {
        cancel();
        if let Some(window) = self.0.get_webview_window("capture-select") {
            let _ = window.destroy();
        }
    }
}

pub(super) async fn select(
    app: tauri::AppHandle,
    bytes: Vec<u8>,
    hint: String,
) -> Result<Option<Vec<u8>>> {
    super::check_snapshot_dimensions(&bytes)?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    let token = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    *SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("Aufnahme nicht verfügbar."))? = Some(Session {
        token: token.clone(),
        frame: Some(CaptureFrame {
            token,
            width,
            height,
            encoded: base64::engine::general_purpose::STANDARD.encode(&bytes),
            hint,
        }),
        sender,
    });
    let _guard = Guard(app.clone());
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "capture-select",
        tauri::WebviewUrl::App("capture-area.html".into()),
    )
    .title("DeepL – Bereich auswählen")
    .decorations(false)
    .fullscreen(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()?;
    window.on_window_event(|event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            cancel();
        }
    });
    let _ = window.set_focus();
    let Some(region) = receiver.await? else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || crop(&bytes, region).map(Some)).await?
}

fn crop(bytes: &[u8], region: CaptureRect) -> Result<Vec<u8>> {
    super::check_snapshot_dimensions(bytes)?;
    let image = image::load_from_memory(bytes)?;
    ensure!(
        region.width > 0
            && region.height > 0
            && region
                .x
                .checked_add(region.width)
                .is_some_and(|x| x <= image.width())
            && region
                .y
                .checked_add(region.height)
                .is_some_and(|y| y <= image.height()),
        "Ungültiger Ziehbereich."
    );
    ensure!(
        region.width <= 16_384
            && region.height <= 16_384
            && u64::from(region.width) * u64::from(region.height) <= 16_000_000,
        "Das Bild ist zu groß für die Texterkennung (maximal 16 Millionen Pixel)."
    );
    let cropped = image
        .crop_imm(region.x, region.y, region.width, region.height)
        .to_rgba8();
    let mut output = std::io::Cursor::new(Vec::new());
    cropped.write_to(&mut output, image::ImageFormat::Png)?;
    let bytes = output.into_inner();
    ensure!(
        bytes.len() <= super::MAX_IMAGE_BYTES,
        "Das Bild ist zu groß (maximal 8 MB)."
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_a_large_selected_region_even_if_png_compresses_well() {
        let image = image::RgbaImage::new(7680, 2160);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let region = super::super::CaptureRect {
            x: 0,
            y: 0,
            width: 7680,
            height: 2160,
        };
        assert!(crop(&encoded.into_inner(), region).is_err());
    }
    #[test]
    fn crops_only_requested_pixels_and_rejects_outside_bounds() {
        let mut image = image::RgbaImage::new(6, 4);
        image.put_pixel(2, 1, image::Rgba([10, 20, 30, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let region = super::super::CaptureRect {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        let result = crop(&bytes, region).unwrap();
        let decoded = image::load_from_memory(&result).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (3, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 30, 255]);
        for invalid in [
            super::super::CaptureRect {
                x: 5,
                y: 1,
                width: 3,
                height: 2,
            },
            super::super::CaptureRect {
                x: u32::MAX,
                y: 0,
                width: 4,
                height: 4,
            },
            super::super::CaptureRect {
                x: 0,
                y: 0,
                width: 0,
                height: 2,
            },
        ] {
            assert!(crop(&bytes, invalid).is_err());
        }
    }
}
