use serde::Deserialize;
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowAction {
    Minimize,
    Maximize,
    Close,
    Drag,
}
#[tauri::command]
pub fn main_window_action(
    window: tauri::WebviewWindow,
    action: WindowAction,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    let result = match action {
        WindowAction::Minimize => window.minimize(),
        WindowAction::Maximize => {
            if window.is_maximized().map_err(|e| e.to_string())? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        WindowAction::Close => window.close(),
        WindowAction::Drag => window.start_dragging(),
    };
    result.map_err(|e| e.to_string())
}
