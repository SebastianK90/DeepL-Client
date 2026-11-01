//! Bounded Linux OCR process I/O. The deadline covers blocked stdin and stdout.
use anyhow::{Result, bail, ensure};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Worker(Child, bool);
impl Drop for Worker {
    fn drop(&mut self) {
        if !self.1 {
            return;
        }
        // The child starts its own process group. Kill descendants too, so an
        // inherited pipe cannot keep the scoped I/O threads blocked after timeout.
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        unsafe {
            kill(-(self.0.id() as i32), 9);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(
    mut command: Command,
    input: &[u8],
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>> {
    let started = Instant::now();
    command
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut worker = Worker(
        command.spawn().map_err(|_| {
            anyhow::anyhow!(
                "OCR-Prozess konnte nicht gestartet werden. Bitte Tesseract installieren."
            )
        })?,
        true,
    );
    let mut stdin = worker
        .0
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("OCR-Eingabe fehlt."))?;
    let stdout = worker
        .0
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("OCR-Ausgabe fehlt."))?;
    std::thread::scope(|scope| {
        let (input_tx, input_rx) = mpsc::channel();
        let (output_tx, output_rx) = mpsc::channel();
        scope.spawn(move || {
            let _ = input_tx.send(stdin.write_all(input));
        });
        scope.spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(limit.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = output_tx.send(result);
        });
        let result = (|| {
            let mut written = false;
            let mut output = None;
            let mut status = None;
            loop {
                if let Ok(result) = input_rx.try_recv() {
                    result?;
                    written = true;
                }
                if let Ok(result) = output_rx.try_recv() {
                    let bytes = result?;
                    ensure!(
                        bytes.len() <= limit,
                        "OCR-Ausgabe überschreitet die Größenbegrenzung."
                    );
                    output = Some(bytes);
                }
                if status.is_none() {
                    status = worker.0.try_wait()?;
                }
                if let Some(status) = status {
                    ensure!(status.success(), "Bilderkennung fehlgeschlagen.");
                    if written && let Some(bytes) = output.take() {
                        return Ok(bytes);
                    }
                }
                if started.elapsed() >= timeout {
                    bail!("Zeitlimit für die Bilderkennung überschritten.");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        // Drop before the scope joins: this unblocks both pipe threads on errors.
        // On success the child has exited and both pipes have already completed.
        if result.is_ok() {
            worker.1 = false;
        }
        drop(worker);
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }
    #[test]
    fn concurrent_pipes_do_not_deadlock() {
        let input = vec![b'x'; 512 * 1024];
        let output = run(
            shell("head -c 131072 /dev/zero; cat"),
            &input,
            Duration::from_secs(5),
            1024 * 1024,
        )
        .unwrap();
        assert_eq!(output.len(), input.len() + 131072);
        assert_eq!(&output[131072..], input);
    }
    #[test]
    fn timeout_kills_and_reaps_child_with_blocked_input_and_descendants() {
        let input = vec![0; 1024 * 1024];
        let start = Instant::now();
        let result = run(
            shell("sleep 10 & wait"),
            &input,
            Duration::from_millis(100),
            1024,
        );
        assert!(result.unwrap_err().to_string().contains("Zeitlimit"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn caps_stdout_and_kills_an_infinite_writer() {
        let start = Instant::now();
        let result = run(shell("cat /dev/zero"), &[], Duration::from_secs(5), 1024);
        assert!(result.unwrap_err().to_string().contains("Größenbegrenzung"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn rejects_failure_and_accepts_exact_output_limit() {
        assert!(run(shell("exit 1"), &[], Duration::from_secs(1), 4).is_err());
        assert_eq!(
            run(shell("printf 1234"), &[], Duration::from_secs(1), 4).unwrap(),
            b"1234"
        );
    }
}
