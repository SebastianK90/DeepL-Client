use super::DoubleCopy;
use tauri::AppHandle;

pub fn start(app: AppHandle, ms: u64) {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = portal(app.clone()).await {
                crate::ui::set_hotkey_status(
                    &app,
                    &format!(
                        "Wayland-Portal nicht verfügbar: {e}. Zwischenablage im Fenster einfügen."
                    ),
                );
            }
        });
    } else {
        std::thread::spawn(move || {
            if let Err(e) = x11(&app, ms) {
                crate::ui::set_hotkey_status(&app, &format!("X11-Hotkey nicht verfügbar: {e}"));
            }
        });
    }
}

async fn portal(app: AppHandle) -> anyhow::Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    // Register on ashpd's shared D-Bus connection before its first portal call.
    // Directly launched host binaries/AppImages otherwise have no portal app ID.
    let app_id = app.config().identifier.as_str().try_into()?;
    if let Err(error) = ashpd::register_host_app(app_id).await {
        // Older portals may lack Registry, or a desktop launcher may already
        // have associated this connection. Still attempt the shortcut session.
        eprintln!("Portal-App-ID konnte nicht registriert werden: {error}");
    }
    let proxy = GlobalShortcuts::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    let mut activated = proxy.receive_activated().await?;
    let shortcuts = [
        NewShortcut::new("quick-translate", "Markierten Text übersetzen")
            .preferred_trigger("CTRL+ALT+d"),
    ];
    proxy
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await?
        .response()?;
    crate::ui::set_hotkey_status(
        &app,
        "Wayland: Ctrl+Alt+D übersetzt die Textmarkierung (PRIMARY).",
    );
    while let Some(event) = activated.next().await {
        if event.shortcut_id() == "quick-translate" {
            crate::ui::quick_from_selection(app.clone());
        }
    }
    Ok(())
}

fn x11(app: &AppHandle, ms: u64) -> anyhow::Result<()> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xinput::{self, ConnectionExt},
            xproto::ConnectionExt as _,
        },
    };
    let (conn, screen) = x11rb::connect(None)?;
    conn.xinput_xi_query_version(2, 0)?.reply()?;
    let root = conn.setup().roots[screen].root;
    conn.xinput_xi_select_events(
        root,
        &[xinput::EventMask {
            deviceid: xinput::Device::ALL_MASTER.into(),
            mask: vec![xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE],
        }],
    )?
    .check()?;
    conn.flush()?;
    let setup = conn.setup();
    let mapping = conn
        .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
        .reply()?;
    let width = mapping.keysyms_per_keycode as usize;
    let codes_for = |sym: u32| -> Vec<u32> {
        mapping
            .keysyms
            .chunks(width)
            .enumerate()
            .filter(|(_, syms)| syms.contains(&sym))
            .map(|(i, _)| setup.min_keycode as u32 + i as u32)
            .collect()
    };
    let c_codes = codes_for(0x0063);
    let mut ctrl_codes = codes_for(0xffe3);
    ctrl_codes.extend(codes_for(0xffe4));
    let mut pressed = std::collections::HashSet::new();
    let mut detector = DoubleCopy::new(ms);
    crate::ui::set_hotkey_status(app, &format!("Ctrl+C+C · {ms} ms (X11)"));
    loop {
        let (key, down) = match conn.wait_for_event()? {
            Event::XinputRawKeyPress(e) => (e.detail, true),
            Event::XinputRawKeyRelease(e) => (e.detail, false),
            _ => continue,
        };
        if down {
            pressed.insert(key);
        } else {
            pressed.remove(&key);
        }
        if ctrl_codes.contains(&key) {
            if !down {
                detector.reset();
            }
            continue;
        }
        if detector.event(
            c_codes.contains(&key),
            down,
            ctrl_codes.iter().any(|k| pressed.contains(k)),
            std::time::Instant::now(),
        ) {
            crate::ui::quick_from_clipboard(app.clone());
        }
    }
}
