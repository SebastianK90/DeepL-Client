//! Native UI fixture: no clipboard, translation service or real user profile.
use tauri::Manager;

pub(super) fn open_quick(app: &tauri::AppHandle) {
    if !super::keep_quick_open_for_test()
        || std::env::var_os("DEEPL_CLIENT_TEST_DIR").is_none()
        || std::env::var_os("DEEPL_CLIENT_PLACEMENT_TEST").is_none()
    {
        return;
    }
    reopen(app.clone());
    let app = app.clone();
    let request = std::path::PathBuf::from(std::env::var_os("DEEPL_CLIENT_TEST_DIR").unwrap())
        .join("reopen-quick.request");
    let mut last = None;
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let next = std::fs::read_to_string(&request)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok());
        if next.is_some() && next != last {
            last = next;
            reopen(app.clone());
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn reopen(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if super::kwin_pointer::required() {
            let snapshot = super::kwin_pointer::capture(&app).await.ok();
            if let Ok(mut value) = app.state::<super::AppState>().quick_cursor.lock() {
                *value = snapshot;
            }
        }
        let owner = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(window) = owner.get_webview_window("quick") {
                let _ = window.hide();
                super::position_quick(&owner, &window);
                let _ = window.show();
                let _ = window.set_focus();
            }
        });
    });
}
