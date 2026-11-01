//! Explicit handoff to Teams Personal or an Outlook draft. Never sends messages.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareTarget {
    Teams,
    Outlook,
}
#[derive(Serialize)]
pub struct ShareOutcome {
    pub copied: bool,
    pub prefilled: bool,
    pub desktop: bool,
}
struct Plan {
    url: String,
    prefilled: bool,
    desktop: bool,
}
pub const fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "linux"))
}
fn validate(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty(),
        "Kein Ergebnis zum Teilen vorhanden."
    );
    ensure!(
        text.chars().count() <= 100_000,
        "Der Text ist zu lang zum Teilen."
    );
    Ok(())
}
fn encode(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}
fn plan(target: ShareTarget, text: &str, desktop: bool) -> Plan {
    // Teams Personal ignores message= on a recipient-free chat link. Opening a
    // protocol handler is not evidence that Teams accepted a text draft. Use
    // the clipboard handoff explicitly, including the consumer web fallback.
    if matches!(target, ShareTarget::Teams) {
        return Plan {
            url: if desktop {
                "msteams:/l/chat/0/0"
            } else {
                "https://teams.live.com/v2/"
            }
            .into(),
            prefilled: false,
            desktop,
        };
    }
    let base = if desktop {
        "mailto:?subject=DeepL%20Client"
    } else {
        "https://outlook.office.com/mail/deeplink/compose?subject=DeepL%20Client"
    };
    let url = format!("{base}&body={}", encode(text));
    // Never truncate long text or split UTF-8 escapes to satisfy a protocol limit.
    let prefilled = url.len() <= 1800;
    Plan {
        url: if prefilled { url } else { base.into() },
        prefilled,
        desktop,
    }
}
#[tauri::command]
pub async fn share_text(
    window: tauri::WebviewWindow,
    target: ShareTarget,
    text: String,
) -> Result<ShareOutcome, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    validate(&text).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        crate::clipboard::write_text(text.clone())
            .map_err(|_| "Kopieren fehlgeschlagen".to_string())?;
        let mut draft = plan(target, &text, desktop_available(target));
        if open_external(&draft.url).is_err() {
            if !draft.desktop {
                return Err(
                    "Ziel-App konnte nicht geöffnet werden. Der Text wurde kopiert.".to_string(),
                );
            }
            draft = plan(target, &text, false);
            open_external(&draft.url).map_err(|_| {
                "Ziel-App konnte nicht geöffnet werden. Der Text wurde kopiert.".to_string()
            })?;
        }
        Ok(ShareOutcome {
            copied: true,
            prefilled: draft.prefilled,
            desktop: draft.desktop,
        })
    })
    .await
    .map_err(|_| "Ziel-App konnte nicht geöffnet werden. Der Text wurde kopiert.".to_string())?
}
#[cfg(target_os = "windows")]
fn desktop_available(target: ShareTarget) -> bool {
    use std::ptr::null;
    use windows_sys::Win32::UI::Shell::{
        ASSOCF_IS_PROTOCOL, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW,
    };
    let scheme: Vec<u16> = match target {
        ShareTarget::Teams => "msteams",
        ShareTarget::Outlook => "mailto",
    }
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let mut buffer = [0u16; 512];
    let mut length = buffer.len() as u32;
    let hr = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_FRIENDLYAPPNAME,
            scheme.as_ptr(),
            null(),
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if hr < 0 || length == 0 {
        return false;
    }
    let name =
        String::from_utf16_lossy(&buffer[..(length as usize).min(buffer.len()).saturating_sub(1)]);
    match target {
        ShareTarget::Teams => !name.is_empty(),
        ShareTarget::Outlook => name.to_lowercase().contains("outlook"),
    }
}
#[cfg(target_os = "linux")]
fn desktop_available(target: ShareTarget) -> bool {
    matches!(target, ShareTarget::Teams)
        && std::process::Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/msteams"])
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn desktop_available(_: ShareTarget) -> bool {
    false
}
#[cfg(target_os = "windows")]
pub(crate) fn open_external(url: &str) -> Result<()> {
    use std::ptr::null;
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            windows_sys::w!("open"),
            url.as_ptr(),
            null(),
            null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    ensure!(result as isize > 32, "External launcher failed");
    Ok(())
}
#[cfg(target_os = "linux")]
pub(crate) fn open_external(url: &str) -> Result<()> {
    ensure!(
        std::process::Command::new("xdg-open")
            .arg(url)
            .status()?
            .success(),
        "External launcher failed"
    );
    Ok(())
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn open_external(_: &str) -> Result<()> {
    anyhow::bail!("Unsupported platform")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_teams_does_not_claim_or_attempt_unsupported_draft_prefill() {
        for desktop in [true, false] {
            let p = plan(ShareTarget::Teams, "A private Teams message", desktop);
            assert!(!p.prefilled);
            assert!(reqwest::Url::parse(&p.url).unwrap().query().is_none());
            if !desktop {
                assert_eq!(
                    reqwest::Url::parse(&p.url).unwrap().host_str(),
                    Some("teams.live.com")
                );
            }
        }
    }
    #[test]
    fn unicode_and_query_injection_remain_one_plain_draft_parameter() {
        let text = "Grüße 🌍\n日本語 <b> &users=intruder@example.com # ? % \"";
        for desktop in [true, false] {
            let p = plan(ShareTarget::Outlook, text, desktop);
            assert!(p.prefilled);
            let url = reqwest::Url::parse(&p.url).unwrap();
            let query: Vec<_> = url.query_pairs().collect();
            assert_eq!(query.iter().find(|(k, _)| k == "body").unwrap().1, text);
            assert!(!query.iter().any(|(k, _)| k == "users"));
            assert!(url.fragment().is_none());
        }
    }
    #[test]
    fn long_text_uses_whole_clipboard_instead_of_truncated_links() {
        for target in [ShareTarget::Teams, ShareTarget::Outlook] {
            let p = plan(target, &"日本語🌍".repeat(1000), true);
            assert!(!p.prefilled);
            assert!(p.url.len() < 100);
            assert!(!p.url.contains("message=") && !p.url.contains("body="));
        }
    }
    #[test]
    fn accepts_only_named_destinations_and_valid_bounded_text() {
        assert!(serde_json::from_str::<ShareTarget>("\"file:///evil\"").is_err());
        assert!(validate(" \n\t").is_err());
        assert!(validate("Grüße 🌍").is_ok());
        assert!(validate(&"界".repeat(100_001)).is_err());
    }
}
