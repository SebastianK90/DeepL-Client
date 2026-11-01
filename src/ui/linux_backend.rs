//! Select a backend before GTK initialization; keep Wayland portals available.
use gtk::prelude::*;
use tauri::Manager;

pub(super) fn install_quick_placement(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> tauri::Result<()> {
    let app = app.clone();
    window.with_webview(move |platform| {
        let view = platform.inner();
        if native_language_popover(&view.display()) {
            return;
        }
        if let Some(toplevel) = view
            .toplevel()
            .and_then(|widget| widget.downcast::<gtk::Window>().ok())
        {
            toplevel.connect_map_event(move |_, _| {
                let app = app.clone();
                // GTK/XWayland can restore old geometry while remapping a hidden
                // toplevel. Apply the selected policy after that map completes.
                gtk::glib::idle_add_local_once(move || {
                    if let Some(quick) = app.get_webview_window("quick")
                        && quick.is_visible().unwrap_or(false)
                    {
                        super::position_quick(&app, &quick);
                    }
                });
                gtk::glib::Propagation::Proceed
            });
        }
    })
}

pub(super) fn native_language_popover(display: &gtk::gdk::Display) -> bool {
    display.type_().name().contains("Wayland")
}

pub(super) async fn uses_native_language_popover(
    window: &tauri::WebviewWindow,
) -> Result<bool, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // GTK objects must be inspected on their UI thread. WAYLAND_DISPLAY refers
    // to the desktop session and remains set when this app runs on XWayland.
    window
        .with_webview(move |platform| {
            let _ = sender.send(native_language_popover(&platform.inner().display()));
        })
        .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())
}

fn prefer_x11(desktop: Option<&str>, wayland: bool, x11: bool, requested: Option<&str>) -> bool {
    wayland
        && x11
        && requested.is_none()
        && desktop.is_some_and(|value| value.split(':').any(|d| d.eq_ignore_ascii_case("KDE")))
}

pub(super) fn configure() {
    if prefer_x11(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        std::env::var("GDK_BACKEND").ok().as_deref(),
    ) && x11rb::connect(None).is_ok()
    {
        // GTK toplevel placement and global pointer coordinates require X11.
        // Do not clear WAYLAND_DISPLAY: selection, hotkey and capture portals
        // still belong to the user's Wayland desktop session.
        gtk::gdk::set_allowed_backends("x11");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kde_wayland_uses_xwayland_for_global_window_placement() {
        assert!(prefer_x11(Some("KDE"), true, true, None));
        assert!(prefer_x11(Some("KDE:PLASMA"), true, true, None));
    }

    #[test]
    fn other_desktops_and_explicit_backends_are_preserved() {
        assert!(!prefer_x11(Some("GNOME"), true, true, None));
        assert!(!prefer_x11(Some("KDE"), true, false, None));
        assert!(!prefer_x11(Some("KDE"), false, true, None));
        assert!(!prefer_x11(Some("KDE"), true, true, Some("wayland")));
    }

    #[test]
    #[ignore = "Requires a KDE Wayland session with XWayland and no explicit GDK_BACKEND"]
    fn native_xwayland_window_can_restore_and_apply_pointer_position() {
        use gtk::prelude::*;
        configure();
        gtk::init().expect("GTK display");
        let display = gtk::gdk::Display::default().expect("Default display");
        assert!(
            display.type_().name().contains("X11"),
            "Expected XWayland backend"
        );
        assert!(
            !native_language_popover(&display),
            "XWayland must use the independently positioned language window, even in a Wayland desktop session"
        );
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("DeepL Client placement regression fixture");
        window.set_decorated(false);
        window.set_default_size(492, 292);
        let settle = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
            while std::time::Instant::now() < deadline {
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        window.show_all();
        settle();
        window.move_(160, 120);
        settle();
        let saved = window.position();
        assert_eq!(saved, (160, 120));
        window.hide();
        window.show_all();
        window.move_(saved.0, saved.1);
        settle();
        assert_eq!(window.position(), saved, "Restore must survive hide/show");
        let (_, x, y) = display
            .default_seat()
            .unwrap()
            .pointer()
            .unwrap()
            .position();
        // Match Quick Assist's pointer offset and work-area clamp.
        let area = display.monitor_at_point(x, y).unwrap().workarea();
        let expected = crate::core::placement::place_quick(
            crate::core::config::QuickPositionPolicy::MouseCursor,
            Some(crate::core::config::SavedPosition {
                x: x + 12,
                y: y + 12,
            }),
            crate::core::placement::WorkArea {
                x: area.x(),
                y: area.y(),
                width: area.width() as u32,
                height: area.height() as u32,
            },
            (492, 292),
        );
        window.move_(expected.x, expected.y);
        settle();
        assert_eq!(window.position(), (expected.x, expected.y));
        window.close();
    }
}
