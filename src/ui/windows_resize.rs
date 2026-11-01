//! Resize without entering WebView2's native mouse-capture/modal-loop path.
use super::{AppState, keep_quick_open_for_test};
use crate::core::window_size::{ResizeEdge, ResizeRect, quick_resize_rect};
use std::sync::atomic::Ordering;

pub(super) async fn start(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    edge: ResizeEdge,
) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    if state.quick_dragging.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = tokio::task::spawn_blocking(move || unsafe {
        use windows_sys::Win32::{
            Foundation::{POINT, RECT},
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
                WindowsAndMessaging::*,
            },
        };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut origin = POINT { x: 0, y: 0 };
        if GetWindowRect(hwnd as _, &mut rect) == 0 || GetCursorPos(&mut origin) == 0 {
            return Err("Fenstergröße konnte nicht geändert werden.".to_string());
        }
        let initial = ResizeRect {
            x: rect.left,
            y: rect.top,
            width: rect.right.saturating_sub(rect.left),
            height: rect.bottom.saturating_sub(rect.top),
        };
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetClientRect(hwnd as _, &mut client) == 0 {
            return Err("Fenstergröße konnte nicht geändert werden.".to_string());
        }
        let minimum = (
            ((492.0 * scale).round() as i32).saturating_add(
                initial
                    .width
                    .saturating_sub(client.right - client.left)
                    .max(0),
            ),
            ((292.0 * scale).round() as i32).saturating_add(
                initial
                    .height
                    .saturating_sub(client.bottom - client.top)
                    .max(0),
            ),
        );
        let started = std::time::Instant::now();
        // Only follow the user's held button. No synthetic input or focus changes.
        while GetAsyncKeyState(VK_LBUTTON as i32) < 0
            && IsWindowVisible(hwnd as _) != 0
            && GetForegroundWindow() == hwnd as _
            && started.elapsed() < std::time::Duration::from_secs(30)
        {
            let mut cursor = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut cursor) == 0 {
                return Err("Fenstergröße konnte nicht geändert werden.".to_string());
            }
            let resized = quick_resize_rect(
                initial,
                (
                    i64::from(cursor.x) - i64::from(origin.x),
                    i64::from(cursor.y) - i64::from(origin.y),
                ),
                edge,
                minimum,
            );
            if SetWindowPos(
                hwnd as _,
                std::ptr::null_mut(),
                resized.x,
                resized.y,
                resized.width,
                resized.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err("Fenstergröße konnte nicht geändert werden.".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    state.quick_dragging.store(false, Ordering::SeqCst);
    if !keep_quick_open_for_test() && !window.is_focused().unwrap_or(false) {
        let _ = window.hide();
    }
    result
}
