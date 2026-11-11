//! PDF parsing runs only in a bounded child. Translation/keyring access stays in the parent.
mod limits;
mod protocol;
use anyhow::{Result, ensure};
use protocol::{FAILURE, Frame, INPUT, OUTPUT, REQUEST, RESPONSE};
use std::{
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const IDLE: Duration = Duration::from_secs(30);
pub(crate) const OVERALL: Duration = Duration::from_secs(30 * 60);
struct Worker {
    child: Child,
    reaped: bool,
    _limits: limits::ProcessLimit,
    _directory: tempfile::TempDir,
}
impl Worker {
    fn finished(&mut self) -> Result<Option<std::process::ExitStatus>> {
        #[cfg(target_os = "linux")]
        {
            // Observe exit without reaping. The zombie retains its PID, so group
            // termination cannot accidentally target a newly recycled process ID.
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { status.si_pid() } == 0 {
                return Ok(None);
            }
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let status = self.child.wait()?;
            self.reaped = true;
            Ok(Some(status))
        }
        #[cfg(target_os = "windows")]
        {
            let status = self.child.try_wait()?;
            if status.is_some() {
                self.reaped = true;
            }
            Ok(status)
        }
    }
    fn stop(&mut self) {
        if self.reaped {
            return;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
enum Event {
    Written(Result<()>),
    Read(Result<Frame>),
}
fn receive_text(bytes: Vec<u8>) -> Result<String> {
    let text = String::from_utf8(bytes)?;
    ensure!(
        !text.trim().is_empty() && text.chars().count() <= 5000,
        "Ungültiger PDF-Textbereich."
    );
    Ok(text)
}

pub(super) fn translate(
    bytes: Vec<u8>,
    mut translator: impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    let executable = std::env::current_exe()?;
    run(
        Command::new(executable),
        bytes,
        &mut translator,
        check,
        IDLE,
    )
}
fn run(
    mut command: Command,
    bytes: Vec<u8>,
    translator: &mut impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
    idle: Duration,
) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() <= super::MAX_BYTES && bytes.starts_with(b"%PDF-"),
        "Ungültige PDF-Datei."
    );
    check()?;
    let directory = tempfile::tempdir()?;
    let preserved: Vec<_> = [
        "LD_LIBRARY_PATH",
        "APPDIR",
        "SystemRoot",
        "WINDIR",
        "WINEPREFIX",
        "WINEDEBUG",
        "WINEDLLPATH",
    ]
    .into_iter()
    .filter_map(|key| std::env::var_os(key).map(|value| (key, value)))
    .collect();
    command
        .arg("--pdf-worker")
        .env_clear()
        .envs(preserved)
        .env("DEEPL_PDF_WORKER", "1")
        .env("DEEPL_PDF_ENGINE_CACHE", directory.path().join("engine"))
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    limits::configure(&mut command)?;
    let mut child = command.spawn()?;
    let limit = match limits::attach(&child) {
        Ok(limit) => limit,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let mut worker = Worker {
        child,
        reaped: false,
        _limits: limit,
        _directory: directory,
    };
    let mut stdin = worker
        .child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("PDF-Eingabe fehlt."))?;
    let mut stdout = worker
        .child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("PDF-Ausgabe fehlt."))?;
    std::thread::scope(|scope| {
        let (write_tx, write_rx) = mpsc::sync_channel::<Frame>(1);
        let (events_tx, events_rx) = mpsc::sync_channel(2);
        let writer_events = events_tx.clone();
        scope.spawn(move || {
            while let Ok(frame) = write_rx.recv() {
                let result = protocol::write(&mut stdin, &frame);
                let failed = result.is_err();
                if writer_events.send(Event::Written(result)).is_err() || failed {
                    break;
                }
            }
        });
        scope.spawn(move || {
            loop {
                let result = protocol::read(&mut stdout, &[REQUEST, OUTPUT, FAILURE]);
                let last = !result.as_ref().is_ok_and(|frame| frame.kind == REQUEST);
                if events_tx.send(Event::Read(result)).is_err() || last {
                    break;
                }
            }
        });
        let result = (|| {
            write_tx.send(Frame { kind: INPUT, bytes })?;
            let start = Instant::now();
            let mut deadline = Instant::now();
            let mut pending_write = true;
            let mut pending = None;
            let mut requests = 0usize;
            let mut characters = 0usize;
            loop {
                check()?;
                ensure!(
                    start.elapsed() < OVERALL,
                    "Zeitlimit für die PDF-Verarbeitung überschritten."
                );
                if !pending_write && let Some(Frame { kind, bytes }) = pending.take() {
                    match kind {
                        REQUEST => {
                            let text = receive_text(bytes)?;
                            requests += 1;
                            characters += text.chars().count();
                            ensure!(
                                requests <= 10_000 && characters <= 300_000,
                                "PDF enthält zu viele Textbereiche."
                            );
                            let translated = translator(&text)?;
                            check()?;
                            ensure!(
                                !translated.trim().is_empty() && translated.len() <= 20_000,
                                "Ungültige PDF-Übersetzung."
                            );
                            write_tx.send(Frame {
                                kind: RESPONSE,
                                bytes: translated.into_bytes(),
                            })?;
                            pending_write = true;
                            deadline = Instant::now();
                        }
                        OUTPUT => {
                            ensure!(bytes.starts_with(b"%PDF-"), "Ungültiges Ausgabedokument.");
                            let exit = Instant::now();
                            loop {
                                check()?;
                                if let Some(status) = worker.finished()? {
                                    ensure!(status.success(), "PDF-Prozess fehlgeschlagen.");
                                    return Ok(bytes);
                                }
                                ensure!(
                                    exit.elapsed() < Duration::from_secs(5),
                                    "PDF-Prozess wurde nicht beendet."
                                );
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        }
                        FAILURE => return Err(anyhow::anyhow!("{}", String::from_utf8(bytes)?)),
                        _ => unreachable!(),
                    }
                }
                ensure!(
                    deadline.elapsed() < idle,
                    "Zeitlimit für die PDF-Verarbeitung überschritten."
                );
                match events_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Event::Written(result)) => {
                        result?;
                        ensure!(pending_write, "Ungültiger PDF-Prozesszustand.");
                        pending_write = false;
                        deadline = Instant::now();
                    }
                    Ok(Event::Read(result)) => {
                        let frame = result?;
                        ensure!(pending.is_none(), "Unerwartete PDF-Prozessantwort.");
                        pending = Some(frame);
                        deadline = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                    Err(error) => return Err(error.into()),
                }
            }
        })();
        // Kill/reap before joining scoped pipe threads so blocked reads/writes end.
        drop(write_tx);
        drop(events_rx);
        worker.stop();
        result
    })
}
pub(crate) fn child_entry() -> Result<()> {
    ensure!(
        std::env::var_os("DEEPL_PDF_WORKER").as_deref() == Some(std::ffi::OsStr::new("1")),
        "Ungültiger PDF-Prozessaufruf."
    );
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let input = protocol::read(&mut stdin, &[INPUT])?;
    let result = super::pdf::translate_cancellable(
        input.bytes,
        |text| {
            ensure!(
                text.len() <= 20_000 && text.chars().count() <= 5000,
                "PDF-Text zu groß."
            );
            protocol::write(
                &mut stdout,
                &Frame {
                    kind: REQUEST,
                    bytes: text.as_bytes().to_vec(),
                },
            )?;
            let response = protocol::read(&mut stdin, &[RESPONSE])?;
            Ok(String::from_utf8(response.bytes)?)
        },
        &mut || Ok(()),
    );
    match result {
        Ok(bytes) => {
            ensure!(bytes.len() <= 128 * 1024 * 1024, "PDF-Ausgabe zu groß.");
            protocol::write(
                &mut stdout,
                &Frame {
                    kind: OUTPUT,
                    bytes,
                },
            )?;
        }
        Err(error) => {
            let message = format!("{error:#}").chars().take(500).collect::<String>();
            protocol::write(
                &mut stdout,
                &Frame {
                    kind: FAILURE,
                    bytes: message.into_bytes(),
                },
            )?;
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }
    #[test]
    fn blocked_input_and_output_obey_deadlines() {
        let mut bytes = b"%PDF-".to_vec();
        bytes.resize(512 * 1024, b'x');
        let start = Instant::now();
        let result = run(
            shell("/bin/sleep 10"),
            bytes,
            &mut |_| unreachable!(),
            &mut || Ok(()),
            Duration::from_millis(150),
        );
        assert!(result.unwrap_err().to_string().contains("Zeitlimit"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn cancellation_kills_worker_while_waiting_for_a_reply() {
        let start = Instant::now();
        let result = run(
            shell("/bin/sleep 10"),
            b"%PDF-test".to_vec(),
            &mut |_| unreachable!(),
            &mut || {
                anyhow::ensure!(
                    start.elapsed() < Duration::from_millis(150),
                    "synthetic cancellation"
                );
                Ok(())
            },
            Duration::from_secs(5),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("synthetic cancellation")
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn successful_worker_output_does_not_leave_a_descendant_running() {
        let directory = tempfile::tempdir().unwrap();
        let pidfile = directory.path().join("pid");
        // Consume the 5-byte INPUT header and 9-byte synthetic PDF before replying.
        // Otherwise exiting can race the broker's separate header/body writes.
        let mut command = shell(
            r#"/usr/bin/dd bs=1 count=14 of=/dev/null status=none || exit 43; /bin/sleep 20 & child_pid=$!; printf '%s' "$child_pid" > "$0"; printf '\004\005\000\000\000'; printf '%%PDF-'"#,
        );
        command.arg(&pidfile);
        let result = run(
            command,
            b"%PDF-test".to_vec(),
            &mut |_| unreachable!(),
            &mut || Ok(()),
            Duration::from_secs(2),
        );
        let pid: i32 = std::fs::read_to_string(pidfile).unwrap().parse().unwrap();
        let alive = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with(['Z', 'X']))
        };
        let start = Instant::now();
        while alive() && start.elapsed() < Duration::from_millis(300) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let survived = alive();
        if survived {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        result.unwrap();
        assert!(
            !survived,
            "worker descendant survived successful completion"
        );
    }
    #[test]
    fn child_failure_and_oversized_frames_are_rejected() {
        for script in ["exit 42", r"printf '\004\377\377\377\377'"] {
            assert!(
                run(
                    shell(script),
                    b"%PDF-test".to_vec(),
                    &mut |_| unreachable!(),
                    &mut || Ok(()),
                    Duration::from_secs(2)
                )
                .is_err()
            );
        }
    }
    #[test]
    fn worker_does_not_inherit_custom_environment_secrets() {
        // Finish reading the complete INPUT frame before returning a successful OUTPUT.
        let mut command = shell(
            r#"/usr/bin/dd bs=1 count=14 of=/dev/null status=none || exit 43; if [ -n "$DEEPL_SYNTHETIC_SECRET" ]; then exit 42; fi; printf '\004\005\000\000\000'; printf '%%PDF-'"#,
        );
        command.env("DEEPL_SYNTHETIC_SECRET", "inert fixture");
        let output = run(
            command,
            b"%PDF-test".to_vec(),
            &mut |_| unreachable!(),
            &mut || Ok(()),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(output, b"%PDF-");
    }
}

/// End-to-end fixture: use native parsing in the parent only to create/inspect
/// synthetic test data. Imported user PDFs always take the subprocess route.
pub(crate) fn self_test() -> Result<()> {
    use pdfium_render::prelude::*;
    let library = super::pdf::library()?;
    let mut source = library.create_new_pdf()?;
    let font = source.fonts_mut().helvetica();
    {
        let mut page = source
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())?;
        page.objects_mut().create_text_object(
            PdfPoints::new(50.),
            PdfPoints::new(700.),
            "Hello world",
            font,
            PdfPoints::new(18.),
        )?;
    }
    let mut calls = 0;
    let output = translate(
        source.save_to_bytes()?,
        |text| {
            ensure!(text == "Hello world", "Unerwarteter PDF-Testtext: {text}");
            calls += 1;
            Ok("Hallo Welt".into())
        },
        &mut || Ok(()),
    )?;
    ensure!(
        calls == 1,
        "PDF-Prozess hat den Übersetzungsbroker nicht benutzt."
    );
    let result = library.load_pdf_from_byte_vec(output, None)?;
    let page = result.pages().get(0)?;
    let extracted = page.text()?.all();
    let objects: Vec<_> = page
        .objects()
        .iter()
        .filter_map(|object| object.as_text_object().map(|text| text.text()))
        .collect();
    ensure!(
        matches_self_test_text(&extracted),
        "PDF self-test output mismatch: page text {extracted:?}, text objects {objects:?}"
    );
    println!(
        "PDF subprocess roundtrip passed: isolated parser, parent translation broker, Unicode font rewrite and successful child exit."
    );
    Ok(())
}

// PDFium may map an embedded font's space glyph to U+00A0 on Windows.
fn matches_self_test_text(text: &str) -> bool {
    text.split_whitespace().eq(["Hallo", "Welt"])
}

#[cfg(test)]
mod self_test_checks {
    use super::matches_self_test_text;

    #[test]
    fn roundtrip_accepts_pdfium_space_mappings_but_rejects_changed_words() {
        for text in ["Hallo Welt", "Hallo\u{a0}Welt", "Hallo\r\nWelt"] {
            assert!(matches_self_test_text(text), "valid PDF text: {text:?}");
        }
        for text in [
            "",
            "Hello world",
            "Hallo",
            "HalloWelt",
            "Hallo Welt extra",
            "Hallo Wel t",
        ] {
            assert!(!matches_self_test_text(text), "invalid PDF text: {text:?}");
        }
    }
}
