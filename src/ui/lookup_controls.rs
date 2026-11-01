use super::{AppState, State};
fn main_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Ungültiges Fenster.".into())
    }
}
#[tauri::command]
pub(super) async fn lookup_alternatives(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    request: crate::lookup::LookupRequest,
) -> Result<crate::lookup::AlternativeResponse, String> {
    main_only(&window)?;
    state
        .interactive
        .lookup(request)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(super) async fn apply_word_alternative(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    ticket: String,
    selection: String,
) -> Result<crate::core::Translation, String> {
    main_only(&window)?;
    state
        .interactive
        .apply(&ticket, &selection)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(super) async fn lookup_dictionary(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    request: crate::lookup::dictionary::DictionaryRequest,
) -> Result<crate::lookup::dictionary::DictionaryResponse, String> {
    main_only(&window)?;
    state
        .dictionary
        .lookup(&request)
        .await
        .map_err(|e| e.to_string())
}
