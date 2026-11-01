use super::{AppState, keep_quick_open_for_test, lock};
use crate::core::{
    config::WorkMode,
    placement::WorkArea,
    popup::{Anchor, PopupPlacement, PopupSession, content_height, place_popup},
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WindowEvent};

#[derive(Serialize, Deserialize)]
pub(super) struct NativeChoice {
    pub(super) code: String,
    pub(super) label: String,
}

#[derive(Default)]
pub(super) struct PopupData {
    session: PopupSession,
    mode: WorkMode,
    placement: Option<PopupPlacement>,
    geometry: Option<PopupGeometry>,
}

#[derive(Clone, Copy)]
struct PopupGeometry {
    anchor: Anchor,
    area: WorkArea,
    width: u32,
    scale: f64,
}

#[derive(Serialize)]
pub(super) struct PopupContext {
    revision: u64,
    open: bool,
    mode: WorkMode,
    selected: String,
    ui_language: String,
    theme: String,
    placement: Option<PopupPlacement>,
}

pub(super) fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    let quick = app
        .get_webview_window("quick")
        .expect("configured Quick Assist window");
    let popup = tauri::WebviewWindowBuilder::new(
        app,
        "quick-language",
        tauri::WebviewUrl::App("language-popup.html".into()),
    )
    .title("Quick Assist – Languages")
    .inner_size(280., 450.)
    .visible(false)
    .focused(false)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .parent(&quick)?;
    #[cfg(target_os = "windows")]
    let popup = popup.shadow(false).transparent(false);
    popup.build()?;
    Ok(())
}

#[tauri::command]
pub(super) async fn language_popup_state(
    state: State<'_, AppState>,
) -> Result<PopupContext, String> {
    let popup = lock(&state.language_popup)?;
    let config = lock(&state.config)?;
    Ok(PopupContext {
        revision: popup.session.revision,
        open: popup.session.is_open,
        mode: popup.mode,
        selected: if popup.mode == WorkMode::Write {
            config.write_language.clone()
        } else {
            config.target.clone()
        },
        ui_language: config.ui_language.clone(),
        theme: config.theme.clone(),
        placement: popup.placement,
    })
}

#[tauri::command]
pub(super) async fn open_language_popup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    mode: WorkMode,
    anchor: Anchor,
    choices: Option<Vec<NativeChoice>>,
    placeholder: Option<String>,
) -> Result<u64, String> {
    if window.label() != "quick" {
        return Err("Ungültiges Fenster.".into());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    if [anchor.x, anchor.y, anchor.width, anchor.height]
        .iter()
        .any(|v| !v.is_finite())
        || anchor.x < 0.
        || anchor.y < 0.
        || anchor.width <= 0.
        || anchor.height <= 0.
        || (anchor.x + anchor.width) * scale > f64::from(size.width) + 2.
        || (anchor.y + anchor.height) * scale > f64::from(size.height) + 2.
    {
        return Err("Ungültiger Ziehbereich.".into());
    }
    #[cfg(target_os = "linux")]
    if super::linux_backend::uses_native_language_popover(&window).await?
        && let Some(choices) = choices
    {
        return super::wayland_language::open(
            app,
            window,
            mode,
            anchor,
            choices,
            placeholder.unwrap_or_default(),
        )
        .await;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (choices, placeholder);
    let origin = window.inner_position().map_err(|e| e.to_string())?;
    let physical = Anchor {
        x: f64::from(origin.x) + anchor.x * scale,
        y: f64::from(origin.y) + anchor.y * scale,
        width: anchor.width * scale,
        height: anchor.height * scale,
    };
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let center = (
        physical.x + physical.width / 2.,
        physical.y + physical.height / 2.,
    );
    let monitor = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            center.0 >= f64::from(p.x)
                && center.1 >= f64::from(p.y)
                && center.0 < f64::from(p.x) + f64::from(s.width)
                && center.1 < f64::from(p.y) + f64::from(s.height)
        })
        .or_else(|| monitors.first())
        .ok_or("Kein Bildschirm verfügbar.")?;
    let area = *monitor.work_area();
    let popup_scale = monitor.scale_factor();
    let placement = place_popup(
        physical,
        WorkArea {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        (
            (280. * popup_scale).max(physical.width).round() as u32,
            (450. * popup_scale).round() as u32,
        ),
        (6. * popup_scale).round() as u32,
    );
    let popup = app
        .get_webview_window("quick-language")
        .ok_or("Sprachauswahl nicht verfügbar.")?;
    let owner = app.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let mut opened_revision = None;
        let result = (|| -> Result<u64, String> {
            let state = owner.state::<AppState>();
            let config = lock(&state.config)?.clone();
            let revision = {
                let mut data = lock(&state.language_popup)?;
                data.mode = mode;
                data.placement = Some(placement);
                data.geometry = Some(PopupGeometry {
                    anchor: physical,
                    area: WorkArea {
                        x: area.position.x,
                        y: area.position.y,
                        width: area.size.width,
                        height: area.size.height,
                    },
                    width: placement.width,
                    scale: popup_scale,
                });
                data.session.open()
            };
            opened_revision = Some(revision);
            let context = PopupContext {
                revision,
                open: true,
                mode,
                selected: if mode == WorkMode::Write {
                    config.write_language
                } else {
                    config.target
                },
                ui_language: config.ui_language,
                theme: config.theme,
                placement: Some(placement),
            };
            // All lifecycle transitions and native mutations share this UI-thread
            // operation. Drop state locks before native APIs can re-enter callbacks.
            popup
                .set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
                .and_then(|_| {
                    popup.set_size(tauri::PhysicalSize::new(placement.width, placement.height))
                })
                .and_then(|_| popup.emit("language-popup-opened", &context))
                .and_then(|_| popup.show())
                .and_then(|_| popup.set_focus())
                .map_err(|e| e.to_string())?;
            Ok(revision)
        })();
        if result.is_err() {
            close_expected(&owner, false, opened_revision, false);
        }
        let _ = sender.send(result);
    })
    .map_err(|e| e.to_string())?;
    receiver.await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn resize_language_popup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    revision: u64,
    rows: usize,
    sequence: u64,
) -> Result<(), String> {
    if window.label() != "quick-language" || rows > crate::core::LANGUAGES.len() {
        return Err("Ungültiges Fenster.".into());
    }
    let owner = app.clone();
    app.run_on_main_thread(move || {
        let placement = {
            let state = owner.state::<AppState>();
            let Ok(mut data) = state.language_popup.lock() else {
                return;
            };
            if !data.session.accept_resize(revision, sequence) {
                return;
            }
            let Some(geometry) = data.geometry else {
                return;
            };
            let placement = place_popup(
                geometry.anchor,
                geometry.area,
                (
                    geometry.width,
                    (f64::from(content_height(rows)) * geometry.scale).round() as u32,
                ),
                (6. * geometry.scale).round() as u32,
            );
            data.placement = Some(placement);
            placement
        };
        if let Some(popup) = owner.get_webview_window("quick-language") {
            let result = popup
                .set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
                .and_then(|_| {
                    popup.set_size(tauri::PhysicalSize::new(placement.width, placement.height))
                })
                .and_then(|_| {
                    popup.emit("language-popup-placement", (revision, sequence, placement))
                });
            if let Err(error) = result {
                let _ = owner.emit("notice", error.to_string());
            }
        }
    })
    .map_err(|e| e.to_string())
}

pub(super) fn close(app: &tauri::AppHandle, refocus: bool) {
    close_expected(app, refocus, None, false);
}

fn close_expected(app: &tauri::AppHandle, refocus: bool, expected: Option<u64>, blur: bool) {
    let owner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if blur
            && owner
                .get_webview_window("quick-language")
                .is_some_and(|w| w.is_focused().unwrap_or(false))
        {
            return;
        }
        let dismiss_quick = blur
            && !owner
                .get_webview_window("quick")
                .is_some_and(|w| w.is_focused().unwrap_or(false));
        let changed = {
            let state = owner.state::<AppState>();
            let Ok(mut data) = state.language_popup.lock() else {
                return;
            };
            if !data.session.is_open
                || expected.is_some_and(|revision| !data.session.accepts(revision))
            {
                return;
            }
            data.session.close();
            true
        };
        if !changed {
            return;
        }
        #[cfg(target_os = "linux")]
        super::wayland_language::close();
        if let Some(popup) = owner.get_webview_window("quick-language") {
            let _ = popup.hide();
        }
        if let Some(quick) = owner.get_webview_window("quick") {
            let _ = quick.emit("quick-language-popup-closed", ());
            if dismiss_quick {
                let _ = quick.hide();
            } else if refocus {
                let _ = quick.set_focus();
            }
        }
    });
}

#[tauri::command]
pub(super) async fn close_language_popup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    revision: Option<u64>,
) -> Result<(), String> {
    if !["quick", "quick-language"].contains(&window.label()) {
        return Err("Ungültiges Fenster.".into());
    }
    close_expected(&app, true, revision, false);
    Ok(())
}

#[tauri::command]
pub(super) async fn choose_quick_language(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    revision: u64,
    code: String,
) -> Result<(), String> {
    if window.label() != "quick-language" {
        return Err("Ungültiges Fenster.".into());
    }
    choose(&app, revision, code)
}

pub(super) fn choose(app: &tauri::AppHandle, revision: u64, code: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mode;
    let next = {
        let popup = lock(&state.language_popup)?;
        if !popup.session.accepts(revision) {
            return Err("Diese Quick-Assist-Sitzung ist nicht mehr aktuell.".into());
        }
        mode = popup.mode;
        if code == "AUTO"
            || !crate::core::LANGUAGES.contains(&code.as_str())
            || (mode == WorkMode::Write && !crate::core::WRITE_LANGUAGES.contains(&code.as_str()))
        {
            return Err("Ungültige Zielsprache.".into());
        }
        let mut config = lock(&state.config)?;
        let mut next = config.clone();
        if mode == WorkMode::Write {
            next.write_language = code;
        } else {
            next.target = code;
        }
        state
            .storage
            .save_config(&next)
            .map_err(|e| e.to_string())?;
        *config = next.clone();
        next
    };
    close_expected(app, true, Some(revision), false);
    let _ = app.emit("settings-changed", &next);
    if let Some(quick) = app.get_webview_window("quick") {
        let _ = quick.emit("quick-language-selected", mode);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn abort_native_open(app: &tauri::AppHandle, revision: u64) {
    close_expected(app, false, Some(revision), false);
}

#[cfg(target_os = "linux")]
pub(super) fn close_native_revision(app: &tauri::AppHandle, revision: u64) {
    let focused = app
        .get_webview_window("quick")
        .is_some_and(|w| w.is_focused().unwrap_or(false));
    close_expected(app, focused, Some(revision), !focused);
}

#[cfg(target_os = "linux")]
pub(super) fn open_native(app: &tauri::AppHandle, mode: WorkMode) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let mut popup = lock(&state.language_popup)?;
    popup.mode = mode;
    popup.placement = None;
    popup.geometry = None;
    Ok(popup.session.open())
}

/// Quick and its popup form one focus group. A true outside click dismisses both.
pub(super) fn on_window_event(window: &tauri::Window, event: &WindowEvent) -> bool {
    let app = window.app_handle();
    if window.label() == "quick" {
        // GTK forwards Wayland popover configure events through the owner. They
        // are not an owner move/resize and must not dismiss the anchored picker.
        #[cfg(target_os = "linux")]
        if super::wayland_language::is_open()
            && matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
        {
            return true;
        }

        #[cfg(all(target_os = "linux", debug_assertions))]
        if matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::Focused(_)
        ) {
            eprintln!(
                "Quick event {event:?}; picker open={}",
                app.state::<AppState>()
                    .language_popup
                    .lock()
                    .is_ok_and(|p| p.session.is_open)
            );
        }
        if matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::CloseRequested { .. }
        ) {
            close(app, false);
        }
        if matches!(event, WindowEvent::Focused(false)) {
            return app
                .state::<AppState>()
                .language_popup
                .lock()
                .is_ok_and(|p| p.session.is_open);
        }
    }
    if window.label() != "quick-language" {
        return false;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        close(app, true);
    }
    if matches!(event, WindowEvent::Focused(false)) && !keep_quick_open_for_test() {
        let app = app.clone();
        let revision = app
            .state::<AppState>()
            .language_popup
            .lock()
            .map(|p| p.session.revision)
            .unwrap_or(0);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            if !app
                .state::<AppState>()
                .language_popup
                .lock()
                .is_ok_and(|p| p.session.accepts(revision))
            {
                return;
            }
            if app
                .get_webview_window("quick-language")
                .is_some_and(|w| w.is_focused().unwrap_or(false))
            {
                return;
            }
            close_expected(&app, false, Some(revision), true);
        });
    }
    true
}
