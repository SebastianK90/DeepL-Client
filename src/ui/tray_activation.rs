//! Linux primary tray activation through the existing StatusNotifierItem.
use gtk::{gio, glib, prelude::*};

const ITEM_PATH: &str = "/org/ayatana/NotificationItem/tray_icon_tray_app_status";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

fn is_activation(message: &gio::DBusMessage, incoming: bool) -> bool {
    incoming
        && message.message_type() == gio::DBusMessageType::MethodCall
        && message.path().as_deref() == Some(ITEM_PATH)
        && message.interface().as_deref() == Some(ITEM_INTERFACE)
        && message.member().as_deref() == Some("Activate")
        && message
            .body()
            .and_then(|body| body.get::<(i32, i32)>())
            .is_some()
}

pub(super) fn install(app: &tauri::AppHandle) -> Result<(), glib::Error> {
    let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)?;
    let app = app.clone();
    // AppIndicator holds the same shared session connection for the process's
    // lifetime. Forward menu/scroll traffic and handle only our primary action.
    connection.add_filter(move |connection, message, incoming| {
        if is_activation(message, incoming) {
            let main_app = app.clone();
            if app
                .run_on_main_thread(move || super::show_main(&main_app))
                .is_ok()
            {
                if !message
                    .flags()
                    .contains(gio::DBusMessageFlags::NO_REPLY_EXPECTED)
                {
                    let reply = message.new_method_reply();
                    reply.set_body(&().to_variant());
                    let _ = connection.send_message(&reply, gio::DBusSendMessageFlags::NONE);
                }
                return None;
            }
        }
        Some(message.clone())
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str, interface: &str, member: &str) -> gio::DBusMessage {
        let message = gio::DBusMessage::new_method_call(None, path, Some(interface), member);
        message.set_body(&(0i32, 0i32).to_variant());
        message
    }

    #[test]
    fn only_incoming_primary_activation_for_our_tray_is_handled() {
        let valid = request(ITEM_PATH, ITEM_INTERFACE, "Activate");
        assert!(is_activation(&valid, true));
        assert!(!is_activation(&valid, false));
        for member in ["ContextMenu", "Scroll", "SecondaryActivate"] {
            assert!(!is_activation(
                &request(ITEM_PATH, ITEM_INTERFACE, member),
                true
            ));
        }
        assert!(!is_activation(
            &request("/other/tray", ITEM_INTERFACE, "Activate"),
            true
        ));
        assert!(!is_activation(
            &request(ITEM_PATH, "other.Interface", "Activate"),
            true
        ));
        valid.set_body(&("invalid",).to_variant());
        assert!(!is_activation(&valid, true));
        let signal = gio::DBusMessage::new_signal(ITEM_PATH, ITEM_INTERFACE, "Activate");
        signal.set_body(&(0i32, 0i32).to_variant());
        assert!(!is_activation(&signal, true));
    }
}
