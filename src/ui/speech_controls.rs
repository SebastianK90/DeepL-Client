use super::{AppState, lock};
use tauri::State;

pub(super) struct SpeechRequests {
    session: u64,
    sequence: u64,
    cancel: tokio::sync::watch::Sender<()>,
}
impl Default for SpeechRequests {
    fn default() -> Self {
        Self {
            session: 0,
            sequence: 0,
            cancel: tokio::sync::watch::channel(()).0,
        }
    }
}
impl SpeechRequests {
    fn begin(&mut self) -> u64 {
        self.cancel.send_replace(());
        self.session = self.session.wrapping_add(1).max(1);
        self.sequence = 0;
        self.session
    }
    fn advance(&mut self, session: u64, sequence: u64) -> bool {
        if session != self.session || sequence <= self.sequence {
            return false;
        }
        self.sequence = sequence;
        self.cancel.send_replace(());
        true
    }
}
#[tauri::command]
pub(super) fn begin_speech_session(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    Ok(lock(&state.speech_requests)?.begin())
}
#[tauri::command]
pub(super) fn cancel_speech(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session: u64,
    sequence: u64,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    lock(&state.speech_requests)?.advance(session, sequence);
    Ok(())
}
#[tauri::command]
pub(super) async fn synthesize_speech(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session: u64,
    sequence: u64,
    text: String,
    language: String,
) -> Result<crate::speech::SpeechOutput, String> {
    run_speech(window, state, session, sequence, text, language, None).await
}
#[tauri::command]
pub(super) async fn stream_speech(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session: u64,
    sequence: u64,
    text: String,
    language: String,
    on_audio: tauri::ipc::Channel<crate::speech::SpeechEvent>,
) -> Result<crate::speech::SpeechOutput, String> {
    run_speech(
        window,
        state,
        session,
        sequence,
        text,
        language,
        Some(on_audio),
    )
    .await
}
async fn run_speech(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    session: u64,
    sequence: u64,
    text: String,
    language: String,
    on_audio: Option<tauri::ipc::Channel<crate::speech::SpeechEvent>>,
) -> Result<crate::speech::SpeechOutput, String> {
    if window.label() != "main" {
        return Err("Ungültiges Fenster.".into());
    }
    let mut cancellation = {
        let mut requests = lock(&state.speech_requests)?;
        if !requests.advance(session, sequence) {
            return Err("Vorlesen abgebrochen.".into());
        }
        requests.cancel.subscribe()
    };
    let send = |event| {
        on_audio
            .as_ref()
            .expect("stream channel present")
            .send(event)
            .map_err(|_| anyhow::anyhow!("Vorlesen abgebrochen."))
    };
    let sink = on_audio.as_ref().map(|_| {
        &send as &(dyn Fn(crate::speech::SpeechEvent) -> anyhow::Result<()> + Send + Sync)
    });
    tokio::select! {biased;
        _=cancellation.changed()=>Err("Vorlesen abgebrochen.".into()),
        output=state.speech.synthesize_stream(&text,&language,sink)=>output.map_err(|e|e.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stale_stop_and_old_documents_cannot_cancel_new_speech() {
        let mut requests = SpeechRequests::default();
        let session = requests.begin();
        assert!(requests.advance(session, 2));
        let mut old = requests.cancel.subscribe();
        assert!(!requests.advance(session, 1));
        assert!(!old.has_changed().unwrap());
        assert!(requests.advance(session, 3));
        assert!(old.changed().await.is_ok());
        let new = requests.begin();
        assert!(!requests.advance(session, 100));
        assert!(requests.advance(new, 1));
    }
}
