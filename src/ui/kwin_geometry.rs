//! KDE geometry lookup limited to this process's Quick Assist window.
use gtk::{gio, glib, prelude::*};
use std::sync::Mutex;
static WINDOW_ID: Mutex<Option<String>> = Mutex::new(None);

fn call(
    connection: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    value: &str,
) -> Option<glib::Variant> {
    connection
        .call_sync(
            Some("org.kde.KWin"),
            path,
            interface,
            method,
            Some(&(value,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            500,
            None::<&gio::Cancellable>,
        )
        .ok()
}
fn own_geometry(connection: &gio::DBusConnection, id: &str) -> Option<(f64, f64)> {
    let result = call(connection, "/KWin", "org.kde.KWin", "getWindowInfo", id)?;
    if result.n_children() != 1 {
        return None;
    }
    let dict = glib::VariantDict::new(Some(&result.child_value(0)));
    if dict.lookup::<i32>("pid").ok().flatten()? != std::process::id() as i32
        || dict.lookup::<String>("caption").ok().flatten()? != "Quick Assist"
    {
        return None;
    }
    let x = dict.lookup::<f64>("x").ok().flatten()?;
    let y = dict.lookup::<f64>("y").ok().flatten()?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}
pub(super) fn quick_origin() -> Option<(f64, f64)> {
    let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
    let cached = WINDOW_ID.lock().ok()?.clone();
    if let Some(id) = cached
        && let Some(geometry) = own_geometry(&connection, &id)
    {
        return Some(geometry);
    }
    let result = call(
        &connection,
        "/WindowsRunner",
        "org.kde.krunner1",
        "Match",
        "Quick Assist",
    )?;
    if result.n_children() != 1 {
        return None;
    }
    let rows = result.child_value(0);
    for index in 0..rows.n_children() {
        let row = rows.child_value(index);
        if row.n_children() < 2 {
            continue;
        }
        if row.child_value(1).get::<String>().as_deref() != Some("Quick Assist") {
            continue;
        }
        let key = row.child_value(0).get::<String>()?;
        let id = key.strip_prefix("0_")?;
        if let Some(geometry) = own_geometry(&connection, id) {
            *WINDOW_ID.lock().ok()? = Some(id.into());
            return Some(geometry);
        }
    }
    None
}
