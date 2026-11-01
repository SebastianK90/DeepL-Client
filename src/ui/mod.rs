use crate::{
    core::{
        Translation, TranslationRequest,
        config::{
            Config, ProviderMode, QuickPositionPolicy, SavedPosition, Storage, WorkMode, key_entry,
        },
        history::{self, HistoryEntry},
        placement::{WorkArea, place_quick},
        quick_session::QuickSession,
    },
    provider::{TranslationProvider, api::ApiProvider, web::WebProvider},
};
use anyhow::Result;
use serde::Serialize;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tauri::{Emitter, Manager, State, WindowEvent};
mod documents;
#[cfg(target_os = "linux")]
mod dropped_files;
#[cfg(target_os = "linux")]
mod kwin_geometry;
#[cfg(target_os = "linux")]
mod kwin_pointer;
mod language_popup;
#[cfg(target_os = "linux")]
mod linux_backend;
mod lookup_controls;
mod quick_reader;
mod speech_controls;
#[cfg(all(target_os = "linux", debug_assertions))]
mod test_fixture;
#[cfg(target_os = "linux")]
mod tray_activation;
mod tray_click;
#[cfg(target_os = "linux")]
mod wayland_language;
mod window_controls;
#[cfg(target_os = "windows")]
mod windows_resize;

pub struct AppState {
    config: Mutex<Config>,
    history: Mutex<Vec<HistoryEntry>>,
    storage: Storage,
    web: WebProvider,
    busy: tokio::sync::Semaphore,
    hotkey_status: Mutex<String>,
    quick_text: Mutex<String>,
    quick_session: Mutex<QuickSession>,
    replacement_target: Mutex<Option<crate::replacement::Target>>,
    replacement_busy: tokio::sync::Semaphore,
    move_revision: AtomicU64,
    quick_read_revision: AtomicU64,
    quick_dragging: AtomicBool,
    #[cfg(target_os = "linux")]
    quick_cursor: Mutex<Option<kwin_pointer::Snapshot>>,
    language_popup: Mutex<language_popup::PopupData>,
    speech: crate::speech::SpeechClient,
    speech_requests: Mutex<speech_controls::SpeechRequests>,
    interactive: crate::lookup::InteractiveClient,
    dictionary: crate::lookup::dictionary::DictionaryClient,
}

#[derive(Serialize)]
struct Bootstrap {
    share_supported: bool,
    inline_quick_languages: bool,
    config: Config,
    history: Vec<HistoryEntry>,
    hotkey_status: String,
    quick_text: String,
    key_present: bool,
}

fn lock<'a, T>(mutex: &'a Mutex<T>) -> Result<std::sync::MutexGuard<'a, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Interner Zustand nicht verfügbar. Anwendung neu starten.".into())
}

#[tauri::command]
async fn bootstrap(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Bootstrap, String> {
    #[cfg(target_os = "linux")]
    let inline_quick_languages = linux_backend::uses_native_language_popover(&window).await?;
    #[cfg(not(target_os = "linux"))]
    let inline_quick_languages = {
        let _ = window;
        false
    };
    let present =
        tokio::task::spawn_blocking(|| key_entry().and_then(|e| Ok(e.get_password()?)).is_ok())
            .await
            .unwrap_or(false);
    Ok(Bootstrap {
        share_supported: crate::share::supported(),
        inline_quick_languages,
        config: lock(&state.config)?.clone(),
        history: lock(&state.history)?.clone(),
        hotkey_status: lock(&state.hotkey_status)?.clone(),
        quick_text: lock(&state.quick_text)?.clone(),
        key_present: present,
    })
}

#[tauri::command]
async fn translate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: TranslationRequest,
    quick_session: Option<u64>,
) -> Result<Translation, String> {
    process_text(app, state, request, WorkMode::Translate, quick_session).await
}

#[tauri::command]
async fn write(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: TranslationRequest,
    quick_session: Option<u64>,
) -> Result<Translation, String> {
    process_text(app, state, request, WorkMode::Write, quick_session).await
}

async fn process_text(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: TranslationRequest,
    mode: WorkMode,
    quick_session: Option<u64>,
) -> Result<Translation, String> {
    if mode == WorkMode::Write {
        request.validate_write()?;
    } else {
        request.validate()?;
    }
    let _permit = state
        .busy
        .try_acquire()
        .map_err(|_| "Eine Textverarbeitung läuft bereits.".to_string())?;
    remember_mode(app.clone(), state.clone(), mode)?;
    let config = lock(&state.config)?.clone();
    let api;
    let provider: &dyn TranslationProvider = if config.provider == ProviderMode::Web {
        &state.web
    } else {
        api = ApiProvider::new(config.provider).map_err(|e| e.to_string())?;
        &api
    };
    let result = match mode {
        WorkMode::Translate => provider.translate(&request).await,
        WorkMode::Write => provider.write(&request).await,
    }
    .map_err(|e| e.to_string())?;
    if let Some(id) = quick_session {
        lock(&state.quick_session)?.complete(id, request.clone(), result.clone(), mode);
    }
    if config.history_enabled {
        let mut entries = lock(&state.history)?;
        let mut next = entries.clone();
        history::push(&mut next, request, result.clone(), mode);
        // A disk failure must not discard a successful (possibly billed) translation.
        match history::save(&state.storage, &next) {
            Ok(()) => {
                *entries = next;
                let _ = app.emit("history-changed", ());
            }
            Err(_) => {
                let _ = app.emit(
                    "notice",
                    "Übersetzung erfolgreich, Verlauf konnte nicht gespeichert werden.",
                );
            }
        }
    }
    Ok(result)
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut config: Config,
) -> Result<(), String> {
    let mut current = lock(&state.config)?;
    // Preference writes from another window must never revert a newly selected mode.
    config.last_mode = current.last_mode;
    config.instance_id = current.instance_id;
    config.quick_position = current.quick_position;
    state
        .storage
        .save_config(&config)
        .map_err(|e| e.to_string())?;
    *current = config.clone();
    drop(current);
    update_tray(&app, config.provider);
    let _ = app.emit("settings-changed", &config);
    Ok(())
}

#[tauri::command]
fn remember_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: WorkMode,
) -> Result<(), String> {
    let mut current = lock(&state.config)?;
    if current.last_mode == mode {
        return Ok(());
    }
    let mut next = current.clone();
    next.last_mode = mode;
    state
        .storage
        .save_config(&next)
        .map_err(|e| e.to_string())?;
    *current = next.clone();
    drop(current);
    let _ = app.emit("settings-changed", next);
    Ok(())
}

#[tauri::command]
fn begin_quick(state: State<'_, AppState>) -> Result<QuickContext, String> {
    let mode = lock(&state.config)?.initial_quick_mode();
    Ok(QuickContext {
        mode,
        session: lock(&state.quick_session)?.id,
        can_replace: lock(&state.replacement_target)?.is_some(),
    })
}

#[derive(Serialize)]
struct QuickContext {
    mode: WorkMode,
    session: u64,
    can_replace: bool,
}

fn keep_quick_open_for_test() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("DEEPL_CLIENT_UI_TEST").is_some()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[tauri::command]
async fn start_quick_resize(
    window: tauri::Window,
    state: State<'_, AppState>,
    edge: Option<crate::core::window_size::ResizeEdge>,
) -> Result<(), String> {
    if window.label() != "quick" {
        return Err("Ungültiges Fenster.".into());
    }
    language_popup::close(window.app_handle(), false);
    let edge = edge.unwrap_or_default();
    #[cfg(target_os = "windows")]
    {
        windows_resize::start(window, state, edge).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        use crate::core::window_size::ResizeEdge;
        let direction = match edge {
            ResizeEdge::North => tauri_runtime::ResizeDirection::North,
            ResizeEdge::NorthEast => tauri_runtime::ResizeDirection::NorthEast,
            ResizeEdge::East => tauri_runtime::ResizeDirection::East,
            ResizeEdge::SouthEast => tauri_runtime::ResizeDirection::SouthEast,
            ResizeEdge::South => tauri_runtime::ResizeDirection::South,
            ResizeEdge::SouthWest => tauri_runtime::ResizeDirection::SouthWest,
            ResizeEdge::West => tauri_runtime::ResizeDirection::West,
            ResizeEdge::NorthWest => tauri_runtime::ResizeDirection::NorthWest,
        };
        window
            .start_resize_dragging(direction)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn start_quick_drag(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    grab_x: f64,
    grab_y: f64,
) -> Result<(), String> {
    if window.label() != "quick" {
        return Err("Ungültiges Fenster.".into());
    }
    language_popup::close(window.app_handle(), false);
    #[cfg(target_os = "windows")]
    {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let size = window.inner_size().map_err(|e| e.to_string())?;
        if !grab_x.is_finite()
            || !grab_y.is_finite()
            || grab_x < 0.0
            || grab_y < 0.0
            || grab_x > f64::from(size.width) / scale
            || grab_y > 100.0
        {
            return Err("Ungültiger Ziehbereich.".into());
        }
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        if state.quick_dragging.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = tokio::task::spawn_blocking(move || unsafe {
            use windows_sys::Win32::{
                Foundation::POINT,
                UI::{
                    Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
                    WindowsAndMessaging::*,
                },
            };
            let offset_x = (grab_x * scale).round() as i32;
            let offset_y = (grab_y * scale).round() as i32;
            let started = std::time::Instant::now();
            // Avoid WebView mouse-capture/native modal-loop dependencies. This does
            // not inject input: it only follows the user's held left mouse button.
            while GetAsyncKeyState(VK_LBUTTON as i32) < 0
                && IsWindowVisible(hwnd as _) != 0
                && GetForegroundWindow() == hwnd as _
                && started.elapsed() < std::time::Duration::from_secs(30)
            {
                let mut point = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut point) == 0 {
                    return Err("Fenster konnte nicht verschoben werden.".into());
                }
                if SetWindowPos(
                    hwnd as _,
                    std::ptr::null_mut(),
                    point.x.saturating_sub(offset_x),
                    point.y.saturating_sub(offset_y),
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                ) == 0
                {
                    return Err("Fenster konnte nicht verschoben werden.".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            Ok(())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        state.quick_dragging.store(false, Ordering::SeqCst);
        // A blur during dragging was deferred, so handle it even if no further
        // focus event follows the end of the gesture.
        if !keep_quick_open_for_test() && !window.is_focused().unwrap_or(false) {
            let _ = window.hide();
        }
        result
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, grab_x, grab_y);
        window.start_dragging().map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn replace_selection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session: u64,
    displayed: String,
) -> Result<(), String> {
    let _permit = state
        .replacement_busy
        .try_acquire()
        .map_err(|_| "Ersetzen läuft bereits.")?;
    let (original, text) = {
        let quick = lock(&state.quick_session)?;
        (
            quick.original.clone(),
            quick
                .replacement(session, &displayed)
                .map_err(str::to_string)?,
        )
    };
    let target = lock(&state.replacement_target)?
        .ok_or("Ersetzen ist auf diesem Desktop nicht verfügbar. Bitte Kopieren verwenden.")?;
    language_popup::close(&app, false);
    if let Some(window) = app.get_webview_window("quick") {
        window.hide().map_err(|e| e.to_string())?;
    }
    let check = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::replacement::replace(target, &original, text, || {
            check
                .state::<AppState>()
                .quick_session
                .lock()
                .is_ok_and(|q| q.id == session && !q.consumed)
        })
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    if result.is_ok() {
        let mut quick = lock(&state.quick_session)?;
        if quick.id == session {
            quick.consumed = true;
        }
    } else if lock(&state.quick_session).is_ok_and(|q| q.id == session && !q.consumed)
        && let Some(window) = app.get_webview_window("quick")
    {
        let _ = window.show();
        let _ = window.set_focus();
    }
    result
}

#[tauri::command]
async fn save_api_key(key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("Bitte einen API-Key eingeben.".into());
    }
    tokio::task::spawn_blocking(move || {
        key_entry()
            .and_then(|e| Ok(e.set_password(&key)?))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_api_key() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let e = key_entry().map_err(|e| e.to_string())?;
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn toggle_favorite(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: uuid::Uuid,
) -> Result<(), String> {
    let mut entries = lock(&state.history)?;
    let mut next = entries.clone();
    let item = next
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or("Verlaufseintrag nicht gefunden")?;
    item.favorite = !item.favorite;
    history::save(&state.storage, &next).map_err(|e| e.to_string())?;
    *entries = next;
    let _ = app.emit("history-changed", ());
    Ok(())
}
#[tauri::command]
fn clear_history(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut entries = lock(&state.history)?;
    history::save(&state.storage, &[]).map_err(|e| e.to_string())?;
    entries.clear();
    let _ = app.emit("history-changed", ());
    Ok(())
}
#[tauri::command]
fn delete_history_entry(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: uuid::Uuid,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    let mut entries = lock(&state.history)?;
    let mut next = entries.clone();
    if !history::remove(&mut next, id) {
        return Err("Verlaufseintrag nicht gefunden".into());
    }
    history::save(&state.storage, &next).map_err(|e| e.to_string())?;
    *entries = next;
    let _ = app.emit("history-changed", ());
    Ok(())
}
#[tauri::command]
fn read_clipboard() -> Result<String, String> {
    crate::clipboard::read_text().map_err(|e| e.to_string())
}
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    crate::clipboard::write_text(text).map_err(|e| e.to_string())
}
#[tauri::command]
fn hide_quick(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    if window.label() == "quick" {
        language_popup::close(window.app_handle(), false);
        lock(&state.quick_session)?.consumed = true;
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
fn open_main(
    app: tauri::AppHandle,
    request: Option<TranslationRequest>,
    translation: Option<Translation>,
    mode: Option<WorkMode>,
    settings: Option<bool>,
) {
    show_main(&app);
    if let Some(request) = request
        && request.validate().is_ok()
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.emit(
            "load-translation",
            (request, translation, mode.unwrap_or_default()),
        );
    }
    if settings.unwrap_or(false)
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.emit("open-settings", ());
    }
}

fn show_main(app: &tauri::AppHandle) {
    language_popup::close(app, false);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
pub fn set_hotkey_status(app: &tauri::AppHandle, message: &str) {
    if let Some(state) = app.try_state::<AppState>()
        && let Ok(mut status) = state.hotkey_status.lock()
    {
        *status = message.into();
    }
    let _ = app.emit("hotkey-status", message);
}
pub fn quick_from_clipboard(app: tauri::AppHandle) {
    quick_from_reader(app, crate::clipboard::read_text, true);
}
#[cfg(target_os = "linux")]
pub fn quick_from_selection(app: tauri::AppHandle) {
    quick_from_reader(app, crate::clipboard::read_primary, false);
}
fn quick_from_reader(app: tauri::AppHandle, read: fn() -> anyhow::Result<String>, retry: bool) {
    let revision = {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        state
            .quick_read_revision
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1)
    };
    language_popup::close(&app, false);
    let target = crate::replacement::capture();
    tauri::async_runtime::spawn(async move {
        let current = || {
            app.try_state::<AppState>()
                .is_some_and(|state| state.quick_read_revision.load(Ordering::SeqCst) == revision)
        };
        #[cfg(target_os = "linux")]
        if kwin_pointer::required() {
            let snapshot = kwin_pointer::capture(&app).await;
            if !current() {
                return;
            }
            if let Some(state) = app.try_state::<AppState>() {
                if snapshot.is_err()
                    && state
                        .config
                        .lock()
                        .is_ok_and(|c| c.quick_position_policy == QuickPositionPolicy::MouseCursor)
                {
                    let _ = app.emit("notice", "Die KDE-Mausposition ist nicht verfügbar. Quick Assist wird mittig geöffnet.");
                }
                if let Ok(mut cursor) = state.quick_cursor.lock()
                    && current()
                {
                    *cursor = snapshot.ok();
                }
            }
        }
        let text = quick_reader::read(read, retry, current).await;
        if !current() {
            return;
        }
        match text {
            Ok(None) => (),
            Ok(Some(text)) => {
                if current()
                    && let Some(state) = app.try_state::<AppState>()
                    && let Ok(mut saved) = state.quick_text.lock()
                    && current()
                {
                    *saved = text.clone();
                }
                if current()
                    && let Some(state) = app.try_state::<AppState>()
                {
                    if let Ok(mut quick) = state.quick_session.lock()
                        && current()
                    {
                        quick.start(text.clone());
                    }
                    if let Ok(mut saved) = state.replacement_target.lock()
                        && current()
                    {
                        *saved = target;
                    }
                }
                if current()
                    && let Some(w) = app.get_webview_window("quick")
                {
                    position_quick(&app, &w);
                    if !current() {
                        return;
                    }
                    let _ = w.emit("quick-text", &text);
                    if !current() {
                        return;
                    }
                    let _ = w.show();
                    if current() {
                        let _ = w.set_focus();
                    }
                }
            }
            Err(e) => {
                if current() {
                    show_main(&app);
                    if current() {
                        let _ = app.emit("notice", e.to_string());
                    }
                }
            }
        }
    });
}

fn position_quick(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    let Some(config) = app
        .try_state::<AppState>()
        .and_then(|s| s.config.lock().ok().map(|c| c.clone()))
    else {
        return;
    };
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    let contains = |m: &&tauri::Monitor, x: f64, y: f64| {
        let p = m.position();
        let s = m.size();
        x >= f64::from(p.x)
            && y >= f64::from(p.y)
            && x < f64::from(p.x) + f64::from(s.width)
            && y < f64::from(p.y) + f64::from(s.height)
    };
    let saved_monitor = if config.quick_position_policy == QuickPositionPolicy::RememberLast {
        config.quick_position.and_then(|p| {
            monitors
                .iter()
                .find(|m| contains(m, f64::from(p.x), f64::from(p.y)))
        })
    } else {
        None
    };
    #[cfg(target_os = "linux")]
    let cursor = if kwin_pointer::required() {
        let native: Vec<_> = monitors
            .iter()
            .map(|m| kwin_pointer::Output {
                name: m.name().cloned().unwrap_or_default(),
                x: f64::from(m.position().x),
                y: f64::from(m.position().y),
                width: f64::from(m.size().width),
                height: f64::from(m.size().height),
            })
            .collect();
        app.try_state::<AppState>()
            .and_then(|s| {
                s.quick_cursor
                    .lock()
                    .ok()
                    .and_then(|snapshot| snapshot.as_ref().and_then(|v| v.map_cursor(&native)))
            })
            .map(|(x, y)| tauri::PhysicalPosition::new(x, y))
    } else {
        window.cursor_position().ok()
    };
    #[cfg(not(target_os = "linux"))]
    let cursor = window.cursor_position().ok();
    let monitor = saved_monitor
        .or_else(|| cursor.and_then(|p| monitors.iter().find(|m| contains(m, p.x, p.y))))
        .or_else(|| monitors.first());
    let Some(monitor) = monitor else {
        return;
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    // Preserve the user's current size when reopening, clamped to this monitor.
    let current = window
        .inner_size()
        .ok()
        .map(|size| size.to_logical::<f64>(window.scale_factor().unwrap_or(scale)));
    let width = current
        .map_or(492.0, |s| s.width)
        .min(f64::from(area.size.width) / scale);
    let height = current
        .map_or(292.0, |s| s.height)
        .min(f64::from(area.size.height) / scale);
    let size = (
        (width * scale).round() as u32,
        (height * scale).round() as u32,
    );
    let p = place_quick(
        config.quick_position_policy,
        if config.quick_position_policy == QuickPositionPolicy::MouseCursor {
            cursor.map(|p| SavedPosition {
                x: (p.x
                    + if cfg!(target_os = "linux") {
                        0.0
                    } else {
                        12.0 * scale
                    })
                .round() as i32,
                y: (p.y + 12.0 * scale).round() as i32,
            })
        } else {
            config.quick_position
        },
        WorkArea {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        size,
    );
    let _ = window.set_position(tauri::PhysicalPosition::new(p.x, p.y));
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
}

fn save_quick_position(app: tauri::AppHandle, position: tauri::PhysicalPosition<i32>) {
    let state = app.state::<AppState>();
    let revision = state
        .move_revision
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let state = app.state::<AppState>();
        if state.move_revision.load(Ordering::Relaxed) != revision {
            return;
        }
        if let Ok(mut config) = state.config.lock()
            && config.quick_position_policy == QuickPositionPolicy::RememberLast
        {
            let mut next = config.clone();
            next.quick_position = Some(SavedPosition {
                x: position.x,
                y: position.y,
            });
            if state.storage.save_config(&next).is_ok() {
                *config = next;
            } else {
                let _ = app.emit(
                    "notice",
                    "Die Fensterposition konnte nicht gespeichert werden.",
                );
            }
        }
    });
}

fn update_tray(app: &tauri::AppHandle, mode: ProviderMode) {
    if let Some(tray) = app.tray_by_id("status") {
        let language = app
            .state::<AppState>()
            .config
            .lock()
            .map(|c| c.ui_language.clone())
            .unwrap_or_else(|_| "de".into());
        if let Ok(menu) = tray_menu(app, &language) {
            let _ = tray.set_menu(Some(menu));
        }
        let label = match mode {
            ProviderMode::ApiFree => "API Free",
            ProviderMode::ApiPro => "API Pro",
            ProviderMode::Web => tray_words(&language)[3],
        };
        let _ = tray.set_tooltip(Some(format!("DeepL Client · {label}")));
        let color = if mode == ProviderMode::Web {
            [204, 146, 65, 255]
        } else {
            [42, 137, 130, 255]
        };
        let _ = tray.set_icon(Some(tauri::image::Image::new_owned(
            tray_pixels(color),
            32,
            32,
        )));
    }
}
fn tray_words(language: &str) -> [&'static str; 4] {
    match language {
        "en" => [
            "Open translator",
            "Clipboard · Translate / Write",
            "Quit",
            "Web (experimental)",
        ],
        "cs" => [
            "Otevřít překladač",
            "Schránka · Překlad / Write",
            "Ukončit",
            "Web (experimentální)",
        ],
        "fr" => [
            "Ouvrir le traducteur",
            "Presse-papiers · Traduction / Write",
            "Quitter",
            "Web (expérimental)",
        ],
        "ja" => [
            "翻訳画面を開く",
            "クリップボード · 翻訳 / Write",
            "終了",
            "Web（試験的）",
        ],
        "it" => [
            "Apri traduttore",
            "Appunti · Traduzione / Write",
            "Esci",
            "Web (sperimentale)",
        ],
        _ => [
            "Übersetzer öffnen",
            "Zwischenablage · Translate / Write",
            "Beenden",
            "Web (experimentell)",
        ],
    }
}
fn tray_menu(
    app: &tauri::AppHandle,
    language: &str,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem};
    let words = tray_words(language);
    let open = MenuItem::with_id(app, "open", words[0], true, None::<&str>)?;
    let quick = MenuItem::with_id(app, "quick", words[1], true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", words[2], true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quick, &quit])
}
fn tray_pixels(color: [u8; 4]) -> Vec<u8> {
    (0..32)
        .flat_map(|y: i32| {
            (0..32).flat_map(move |x: i32| {
                if (x - 16).pow(2) + (y - 16).pow(2) < 180 {
                    color
                } else {
                    [0, 0, 0, 0]
                }
            })
        })
        .collect()
}

pub fn run() -> Result<()> {
    #[cfg(target_os = "linux")]
    linux_backend::configure();
    let context = tauri::generate_context!();
    #[cfg(target_os = "windows")]
    let context = {
        let mut context = context;
        if let Some(quick) = context
            .config_mut()
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "quick")
        {
            // Frameless fixed-size windows otherwise acquire a native white frame
            // and client-area offsets. The web content already paints an opaque panel.
            quick.shadow = false;
            quick.transparent = false;
            // Use our focus-safe resize regions instead of native modal sizing.
            quick.resizable = false;
        }
        context
    };
    let storage = Storage::discover()?;
    let config = storage.load_config()?;
    let start_in_tray = config.start_in_tray;
    let entries = history::load(&storage)?;
    let hotkey_ms = config.hotkey_ms;
    let mode = config.provider;
    let state = AppState {
        web: WebProvider::new(config.instance_id)?,
        config: Mutex::new(config),
        history: Mutex::new(entries),
        storage,
        busy: tokio::sync::Semaphore::new(1),
        hotkey_status: Mutex::new("Hotkey wird initialisiert …".into()),
        quick_text: Mutex::new(String::new()),
        quick_session: Mutex::new(QuickSession::default()),
        replacement_target: Mutex::new(None),
        replacement_busy: tokio::sync::Semaphore::new(1),
        move_revision: AtomicU64::new(0),
        quick_read_revision: AtomicU64::new(0),
        quick_dragging: AtomicBool::new(false),
        #[cfg(target_os = "linux")]
        quick_cursor: Mutex::new(None),
        language_popup: Mutex::new(language_popup::PopupData::default()),
        speech: crate::speech::SpeechClient::new()?,
        speech_requests: Mutex::new(speech_controls::SpeechRequests::default()),
        interactive: crate::lookup::InteractiveClient::new()?,
        dictionary: crate::lookup::dictionary::DictionaryClient::new()?,
    };
    tauri::Builder::default()
        .manage(state)
        .manage(documents::Jobs::default())
        .invoke_handler({
            let handlers: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
                Box::new(tauri::generate_handler![
                    documents::translate_document,
                    documents::cancel_document,
                    documents::save_document,
                    crate::embedded_pdfium::third_party_licenses,
                    window_controls::main_window_action,
                    crate::capture::recognize_image,
                    bootstrap,
                    translate,
                    write,
                    remember_mode,
                    begin_quick,
                    save_settings,
                    save_api_key,
                    delete_api_key,
                    toggle_favorite,
                    clear_history,
                    delete_history_entry,
                    read_clipboard,
                    copy_text,
                    crate::share::share_text,
                    crate::capture::capture_screenshot,
                    crate::capture::capture_area_state,
                    crate::capture::capture_area_finish,
                    hide_quick,
                    open_main,
                    start_quick_drag,
                    start_quick_resize,
                    replace_selection,
                    language_popup::language_popup_state,
                    language_popup::open_language_popup,
                    language_popup::close_language_popup,
                    language_popup::choose_quick_language,
                    language_popup::resize_language_popup,
                    speech_controls::begin_speech_session,
                    speech_controls::cancel_speech,
                    speech_controls::synthesize_speech,
                    speech_controls::stream_speech,
                    lookup_controls::lookup_alternatives,
                    lookup_controls::apply_word_alternative,
                    lookup_controls::lookup_dictionary
                ]);
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| handlers(invoke)
        })
        .setup(move |app| {
            if let Some(main) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                dropped_files::install(&main)?;
                let scale = main.scale_factor()?;
                let outer = main.outer_size()?;
                let inner = main.inner_size()?;
                let (width, height) = crate::core::window_size::main_inner_size(
                    scale,
                    (outer.width, outer.height),
                    (inner.width, inner.height),
                );
                main.set_size(tauri::PhysicalSize::new(width, height))?;
            }
            use tauri::tray::TrayIconBuilder;
            let language = app
                .state::<AppState>()
                .config
                .lock()
                .map(|c| c.ui_language.clone())
                .unwrap_or_else(|_| "de".into());
            let menu = tray_menu(app.handle(), &language)?;
            TrayIconBuilder::with_id("status")
                .icon(tauri::image::Image::new_owned(
                    tray_pixels([42, 137, 130, 255]),
                    32,
                    32,
                ))
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if tray_click::opens_main(&event) {
                        show_main(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quick" => quick_from_clipboard(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            #[cfg(target_os = "linux")]
            tray_activation::install(app.handle())?;
            update_tray(app.handle(), mode);
            language_popup::setup(app.handle())?;
            #[cfg(target_os = "linux")]
            if let Some(quick) = app.get_webview_window("quick") {
                linux_backend::install_quick_placement(app.handle(), &quick)?;
            }
            crate::hotkey::start(app.handle().clone(), hotkey_ms);
            // The main window is created hidden to avoid flashing before the
            // saved startup preference and tray initialization are applied.
            if !start_in_tray {
                show_main(app.handle());
            }
            #[cfg(all(target_os = "linux", debug_assertions))]
            test_fixture::open_quick(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if language_popup::on_window_event(window, event) {
                return;
            }
            if window.label() == "quick"
                && let WindowEvent::Moved(position) = event
            {
                save_quick_position(window.app_handle().clone(), *position);
            }
            if window.label() == "quick" && matches!(event, WindowEvent::Focused(false)) {
                if window
                    .state::<AppState>()
                    .quick_dragging
                    .load(Ordering::SeqCst)
                {
                    return;
                }
                // Debug-only UI fixture: desktop automation returns focus to its controller.
                if keep_quick_open_for_test() {
                    return;
                }
                let _ = window.hide();
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "main" {
                    let _ = window.emit("speech-stop", ());
                }
                let _ = window.hide();
            }
        })
        .run(context)?;
    Ok(())
}
