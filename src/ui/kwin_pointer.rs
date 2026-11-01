//! One-shot compositor pointer lookup; XWayland pointer coordinates can be stale.
use gtk::{gio, glib, prelude::*};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

const INTERFACE: &str = "dev.local.DeepLClient.Pointer";
const SCRIPTING: &str = "org.kde.kwin.Scripting";

pub(super) fn required() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|d| d.split(':').any(|v| v.eq_ignore_ascii_case("KDE")))
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Output {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Snapshot {
    pub x: f64,
    pub y: f64,
    pub outputs: Vec<Output>,
}

impl Snapshot {
    pub(super) fn map_cursor(&self, native: &[Output]) -> Option<(f64, f64)> {
        let valid = |output: &Output| {
            [output.x, output.y, output.width, output.height]
                .iter()
                .all(|v| v.is_finite())
                && output.width > 0.0
                && output.height > 0.0
        };
        if !self.x.is_finite()
            || !self.y.is_finite()
            || self.outputs.len() > 64
            || !self.outputs.iter().all(valid)
        {
            return None;
        }
        let source = self.outputs.iter().find(|o| {
            self.x >= o.x && self.x < o.x + o.width && self.y >= o.y && self.y < o.y + o.height
        })?;
        let mut matching = native.iter().filter(|o| o.name == source.name);
        let target = match (matching.next(), matching.next()) {
            (Some(target), None) => target,
            (None, None) if self.outputs.len() == 1 && native.len() == 1 => &native[0],
            _ => return None,
        };
        if !valid(target) {
            return None;
        }
        Some((
            target.x + (self.x - source.x) * target.width / source.width,
            target.y + (self.y - source.y) * target.height / source.height,
        ))
    }
}

pub(super) async fn capture(app: &tauri::AppHandle) -> Result<Snapshot, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        glib::MainContext::default().spawn_local(async move {
            let _ = sender.send(query().await);
        });
    })
    .map_err(|e| e.to_string())?;
    receiver.await.map_err(|e| e.to_string())?
}

async fn call(
    connection: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<glib::Variant, String> {
    connection
        .call_future(
            Some("org.kde.KWin"),
            path,
            interface,
            method,
            args,
            None,
            gio::DBusCallFlags::NONE,
            500,
        )
        .await
        .map_err(|e| e.to_string())
}

async fn query() -> Result<Snapshot, String> {
    let connection = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|e| e.to_string())?;
    let owner = connection
        .call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&("org.kde.KWin",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            500,
        )
        .await
        .map_err(|e| e.to_string())?
        .get::<(String,)>()
        .ok_or("Invalid KWin owner")?
        .0;
    let service = connection.unique_name().ok_or("Missing D-Bus name")?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("deepl-client-pointer-{nonce}");
    let path = format!("/dev/local/DeepLClient/Pointer/p{nonce}");
    let info = gio::DBusNodeInfo::for_xml("<node><interface name='dev.local.DeepLClient.Pointer'><method name='Report'><arg type='s' direction='in'/></method></interface></node>")
        .map_err(|e| e.to_string())?;
    let interface = info
        .lookup_interface(INTERFACE)
        .ok_or("Missing pointer interface")?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let registration = connection
        .register_object(
            &path,
            &interface,
            move |_, source, _, _, method, parameters, invocation| {
                if source != owner || method != "Report" {
                    invocation.return_dbus_error(
                        "dev.local.DeepLClient.Error",
                        "Unexpected pointer sender",
                    );
                    return;
                }
                let result = parameters
                    .get::<(String,)>()
                    .filter(|(s,)| s.len() <= 32768)
                    .ok_or_else(|| "Invalid pointer payload".to_string())
                    .and_then(|(s,)| {
                        serde_json::from_str::<Snapshot>(&s).map_err(|e| e.to_string())
                    });
                if let Ok(mut sender) = sender.lock()
                    && let Some(sender) = sender.take()
                {
                    let _ = sender.send(result);
                }
                invocation.return_value(Some(&().to_variant()));
            },
            |_, _, _, _, _| ().to_variant(),
            |_, _, _, _, _, _| false,
        )
        .map_err(|e| e.to_string())?;
    let result = async {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let script = directory.path().join("pointer.js");
        // No window enumeration, input injection, persistent installation or polling.
        let source = format!("const p=workspace.cursorPos;const r={{x:p.x,y:p.y,outputs:[]}};for(const s of workspace.screens)r.outputs.push({{name:s.name,x:s.geometry.x,y:s.geometry.y,width:s.geometry.width,height:s.geometry.height}});callDBus({},{},{},\"Report\",JSON.stringify(r));",
            serde_json::to_string(service.as_str()).map_err(|e| e.to_string())?,
            serde_json::to_string(&path).map_err(|e| e.to_string())?,
            serde_json::to_string(INTERFACE).map_err(|e| e.to_string())?);
        std::fs::write(&script, source).map_err(|e| e.to_string())?;
        let id = call(&connection, "/Scripting", SCRIPTING, "loadScript", Some(&(script.to_string_lossy().as_ref(), name.as_str()).to_variant()))
            .await?.get::<(i32,)>().ok_or("Invalid script ID")?.0;
        if id < 0 { return Err("KWin rejected pointer query".to_string()); }
        call(&connection, &format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", None).await?;
        match futures_util::future::select(Box::pin(receiver), Box::pin(glib::timeout_future(std::time::Duration::from_millis(500)))).await {
            futures_util::future::Either::Left((report, _)) => report.map_err(|e| e.to_string())?,
            futures_util::future::Either::Right(_) => Err("KWin pointer query timed out".into()),
        }
    }.await;
    // Always unload only our temporary script, including timeout and malformed replies.
    let _ = call(
        &connection,
        "/Scripting",
        SCRIPTING,
        "unloadScript",
        Some(&(name.as_str(),).to_variant()),
    )
    .await;
    let _ = connection.unregister_object(registration);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires KDE and its session-bus scripting service"]
    fn native_compositor_query_returns_current_pointer_and_outputs() {
        let context = glib::MainContext::new();
        let snapshot = context
            .with_thread_default(|| context.block_on(query()))
            .unwrap()
            .expect("KWin pointer query");
        assert!(snapshot.x.is_finite() && snapshot.y.is_finite());
        assert!(!snapshot.outputs.is_empty());
        assert!(snapshot.map_cursor(&snapshot.outputs).is_some());
        eprintln!(
            "Compositor pointer: ({}, {}), {} output(s)",
            snapshot.x,
            snapshot.y,
            snapshot.outputs.len()
        );
    }
    fn output(name: &str, x: f64, y: f64, width: f64, height: f64) -> Output {
        Output {
            name: name.into(),
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn compositor_point_is_scaled_to_xwayland_without_using_stale_x_pointer() {
        let snapshot = Snapshot {
            x: 0.0,
            y: 456.0,
            outputs: vec![output("Virtual-1", 0.0, 0.0, 1703.0, 839.0)],
        };
        let point = snapshot
            .map_cursor(&[output("Virtual-1", 0.0, 0.0, 2554.0, 1258.0)])
            .unwrap();
        assert_eq!(point.0, 0.0);
        assert!((point.1 - 456.0 * 1258.0 / 839.0).abs() < 0.01);
    }
    #[test]
    fn output_names_and_local_offsets_handle_reordered_and_negative_monitors() {
        let snapshot = Snapshot {
            x: -1500.0,
            y: 300.0,
            outputs: vec![
                output("left", -1920.0, 0.0, 1920.0, 1080.0),
                output("right", 0.0, 0.0, 2560.0, 1440.0),
            ],
        };
        let point = snapshot
            .map_cursor(&[
                output("right", 0.0, 0.0, 2560.0, 1440.0),
                output("left", -2880.0, 0.0, 2880.0, 1620.0),
            ])
            .unwrap();
        assert_eq!(point, (-2250.0, 450.0));
    }
    #[test]
    fn invalid_or_unmatched_outputs_never_fall_back_to_wrong_coordinates() {
        let native = [output("other", 0.0, 0.0, 1920.0, 1080.0)];
        let mut snapshot = Snapshot {
            x: 12.0,
            y: 30.0,
            outputs: vec![
                output("one", 0.0, 0.0, 800.0, 600.0),
                output("two", 800.0, 0.0, 800.0, 600.0),
            ],
        };
        assert!(snapshot.map_cursor(&native).is_none());
        snapshot.x = f64::NAN;
        assert!(snapshot.map_cursor(&native).is_none());
        snapshot.x = 12.0;
        snapshot.outputs[0].width = 0.0;
        assert!(snapshot.map_cursor(&native).is_none());
    }
}
