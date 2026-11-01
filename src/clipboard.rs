use anyhow::{Context, Result};
use std::sync::{OnceLock, mpsc};

enum Command {
    Read(mpsc::Sender<Result<String>>),
    #[cfg(target_os = "linux")]
    ReadPrimary(mpsc::Sender<Result<String>>),
    Write(String, mpsc::Sender<Result<()>>),
}
// Keep the clipboard owner alive, especially for X11/Wayland selections.
fn worker() -> &'static mpsc::Sender<Command> {
    static SENDER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut clipboard = None;
            for command in rx {
                if clipboard.is_none() {
                    clipboard = arboard::Clipboard::new().ok();
                }
                match command {
                    Command::Read(reply) => {
                        let result = clipboard
                            .as_mut()
                            .context("Zwischenablage nicht verfügbar")
                            .and_then(|c| c.get_text().context("Kein Text in der Zwischenablage"));
                        let _ = reply.send(result);
                    }
                    #[cfg(target_os = "linux")]
                    Command::ReadPrimary(reply) => {
                        use arboard::{GetExtLinux, LinuxClipboardKind};
                        let result = clipboard.as_mut()
                            .context("Textmarkierung nicht verfügbar")
                            .and_then(|c| c.get().clipboard(LinuxClipboardKind::Primary).text()
                                .context("Keine Textmarkierung verfügbar. Bitte Text in der Quellanwendung markieren."));
                        let _ = reply.send(result);
                    }
                    Command::Write(text, reply) => {
                        let result = clipboard
                            .as_mut()
                            .context("Zwischenablage nicht verfügbar")
                            .and_then(|c| c.set_text(text).context("Kopieren fehlgeschlagen"));
                        let _ = reply.send(result);
                    }
                }
            }
        });
        tx
    })
}

pub fn read_text() -> Result<String> {
    let (tx, rx) = mpsc::channel();
    worker().send(Command::Read(tx))?;
    let text = rx.recv_timeout(std::time::Duration::from_secs(2))??;
    anyhow::ensure!(!text.trim().is_empty(), "Die Zwischenablage ist leer.");
    anyhow::ensure!(
        text.chars().count() <= 5_000,
        "Die Zwischenablage enthält mehr als 5.000 Zeichen."
    );
    Ok(text)
}
pub fn write_text(text: String) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    worker().send(Command::Write(text, tx))?;
    rx.recv_timeout(std::time::Duration::from_secs(2))?
}

#[cfg(target_os = "linux")]
fn validate_primary(text: String) -> Result<String> {
    anyhow::ensure!(
        !text.trim().is_empty(),
        "Keine Textmarkierung verfügbar. Bitte Text in der Quellanwendung markieren."
    );
    anyhow::ensure!(
        text.chars().count() <= 5000,
        "Die Textmarkierung enthält mehr als 5.000 Zeichen."
    );
    Ok(text)
}
#[cfg(target_os = "linux")]
pub fn read_primary() -> Result<String> {
    let (tx, rx) = mpsc::channel();
    worker().send(Command::ReadPrimary(tx))?;
    validate_primary(rx.recv_timeout(std::time::Duration::from_secs(2))??)
}

#[cfg(all(test, target_os = "linux"))]
mod selection_tests {
    use super::*;
    #[test]
    fn empty_selection_never_falls_back_to_clipboard() {
        assert!(validate_primary(String::new()).is_err());
        assert!(validate_primary(" \n\t ".into()).is_err());
        assert_eq!(
            validate_primary(" Grüße 世界 ".into()).unwrap(),
            " Grüße 世界 "
        );
    }
    #[test]
    fn selection_limit_counts_unicode_characters() {
        assert!(validate_primary("界".repeat(5000)).is_ok());
        assert!(validate_primary("界".repeat(5001)).is_err());
    }
}
