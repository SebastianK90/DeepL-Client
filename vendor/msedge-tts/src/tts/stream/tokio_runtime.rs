//! Tokio Async Runtime

use std::{sync::Arc, time::Duration};

use async_tungstenite::{
    WebSocketReceiver, WebSocketSender, WebSocketStream, tokio::ConnectStream,
};
use futures_util::{
    StreamExt,
    io::{AsyncRead, AsyncWrite},
};
use tokio::{sync::Mutex, time::sleep};

use crate::{
    error::Result,
    tts::{
        Payload, SpeechConfig, build_config_message, build_ssml_message,
        stream::SynthesizedResponse, websocket_connect_tokio_async,
    },
};

/// Async TTS Stream Sender
pub struct SenderAsync<T> {
    sender: WebSocketSender<T>,
    can_read: Arc<Mutex<bool>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> SenderAsync<T> {
    /// Synthesize text to speech with a [SpeechConfig] asynchronously.  
    /// **Caution**: One [send](Self::send) corresponds to multiple [read](ReceiverAsync::read). Next [send](Self::send) call will block until there no data to read.
    /// [read](ReceiverAsync::read) will block before you call a [send](Self::send).
    pub async fn send(&mut self, text: &str, config: &SpeechConfig) -> Result<()> {
        while !self.can_send().await {
            sleep(Duration::from_millis(1)).await;
        }
        let mut can_read = self.can_read.lock().await;
        let config_message = build_config_message(config);
        let ssml_message = build_ssml_message(text, config);
        self.sender.send(config_message).await?;
        self.sender.send(ssml_message).await?;
        *can_read = true;
        Ok(())
    }

    /// Check if can send
    pub async fn can_send(&self) -> bool {
        !*self.can_read.lock().await
    }
}

/// Async TTS Stream Reader
pub struct ReceiverAsync<T> {
    receiver: WebSocketReceiver<T>,
    can_read: Arc<Mutex<bool>>,
    turn_start: bool,
    response: bool,
    turn_end: bool,
}

impl<T: AsyncRead + AsyncWrite + Unpin> ReceiverAsync<T> {
    /// Read Synthesized Audio asynchronously.  
    /// **Caution**: One [send](SenderAsync::send) corresponds to multiple [read](Self::read). Next [send](SenderAsync::send) call will block until there no data to read.
    /// [read](Self::read) will block before you call a [send](SenderAsync::send).
    pub async fn read(&mut self) -> Result<Option<SynthesizedResponse>> {
        while !self.can_read().await {
            sleep(Duration::from_millis(1)).await;
        }

        let message = self.receiver.next().await;
        if let Some(message) = message {
            let message = message?;
            // Only speech turn.end completes a request. A transport close must
            // fail immediately, including when no turn.start/response arrived.
            if matches!(message, tungstenite::Message::Close(_)) {
                return Err(crate::error::Error::UnexpectedMessage(
                    "connection closed before speech turn.end".into(),
                ));
            }
            let payload = Payload::process(
                message,
                &mut self.turn_start,
                &mut self.response,
                &mut self.turn_end,
            )?;

            if self.turn_start && self.response && self.turn_end {
                self.turn_start = false;
                self.response = false;
                self.turn_end = false;
                *self.can_read.lock().await = false;
            }

            Ok(payload.map(|payload| payload.into()))
        } else {
            Err(crate::error::Error::UnexpectedMessage(
                "connection ended before speech turn.end".into(),
            ))
        }
    }

    /// Check if can read
    pub async fn can_read(&self) -> bool {
        *self.can_read.lock().await
    }
}

pub(crate) fn split<T: AsyncRead + AsyncWrite + Unpin>(
    websocket: WebSocketStream<T>,
) -> Result<(SenderAsync<T>, ReceiverAsync<T>)> {
    let (sender, receiver) = websocket.split();
    let can_read = Arc::new(Mutex::new(false));
    Ok((
        SenderAsync {
            sender,
            can_read: can_read.clone(),
        },
        ReceiverAsync {
            receiver,
            can_read,
            turn_start: false,
            response: false,
            turn_end: false,
        },
    ))
}

/// Create Async TTS Stream [SenderAsync] and [ReceiverAsync]
pub async fn msedge_tts_split_async()
-> Result<(SenderAsync<ConnectStream>, ReceiverAsync<ConnectStream>)> {
    split(websocket_connect_tokio_async().await?)
}

#[cfg(feature = "proxy")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "tokio-runtime", feature = "proxy"))))]
pub use crate::tts::proxy::tokio_runtime::msedge_tts_split_proxy_async;

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::io::Cursor;
    use tungstenite::protocol::Role;

    async fn fixture(frames: &[(u8, &[u8])]) -> ReceiverAsync<Cursor<Vec<u8>>> {
        let mut wire = Vec::new();
        for (opcode, frame) in frames {
            assert!(frame.len() < 126);
            wire.extend([*opcode, frame.len() as u8]);
            wire.extend_from_slice(frame);
        }
        // The peer closes gracefully, but without the required speech turn.end.
        wire.extend([0x88, 0]);
        let socket = WebSocketStream::from_raw_socket(Cursor::new(wire), Role::Client, None).await;
        let (_, receiver) = split(socket).unwrap();
        *receiver.can_read.lock().await = true;
        receiver
    }

    #[tokio::test]
    async fn peer_close_before_turn_start_is_an_error_instead_of_an_empty_read_loop() {
        let mut receiver = fixture(&[]).await;
        assert!(
            receiver.read().await.is_err(),
            "premature close must stop the consumer read loop"
        );
    }

    #[tokio::test]
    async fn exhausted_transport_is_an_error_instead_of_repeated_empty_reads() {
        let mut receiver = fixture(&[]).await;
        assert!(matches!(
            receiver.receiver.next().await,
            Some(Ok(tungstenite::Message::Close(_)))
        ));
        assert!(
            receiver.read().await.is_err(),
            "transport EOF must stop the consumer read loop"
        );
    }

    #[tokio::test]
    async fn peer_close_after_response_does_not_report_truncated_speech_as_complete() {
        let mut receiver = fixture(&[
            (0x81, b"Path:turn.start\r\n\r\n"),
            (0x81, b"Path:response\r\n\r\n"),
        ])
        .await;
        assert!(receiver.read().await.unwrap().is_none());
        assert!(receiver.read().await.unwrap().is_none());
        assert!(
            receiver.read().await.is_err(),
            "close cannot replace the speech turn.end frame"
        );
    }

    #[tokio::test]
    async fn metadata_audio_and_turn_end_keep_the_stream_readable_until_complete() {
        let mut receiver = fixture(&[
            (0x81, b"Path:turn.start\r\n\r\n"),
            (0x81, b"Path:response\r\n\r\n"),
            (0x81, b"Path:audio.metadata\r\n\r\n{\"Metadata\":[]}"),
            (0x82, b"\x00\x00synthetic-audio"),
            (0x81, b"Path:turn.end\r\n\r\n"),
        ])
        .await;
        assert!(receiver.read().await.unwrap().is_none());
        assert!(receiver.can_read().await);
        assert!(receiver.read().await.unwrap().is_none());
        assert!(matches!(
            receiver.read().await.unwrap(),
            Some(SynthesizedResponse::AudioMetadata(_))
        ));
        assert!(receiver.can_read().await);
        let Some(SynthesizedResponse::AudioBytes(audio)) = receiver.read().await.unwrap() else {
            panic!("audio expected");
        };
        assert_eq!(audio, b"synthetic-audio");
        assert!(receiver.can_read().await);
        assert!(receiver.read().await.unwrap().is_none());
        assert!(!receiver.can_read().await);
    }
}
