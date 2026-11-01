pub mod api;
pub mod web;

use crate::core::{Translation, TranslationRequest};
use async_trait::async_trait;
use reqwest::{Client, Response};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Netzwerkanfrage fehlgeschlagen. Verbindung prüfen und erneut versuchen.")]
    Network(#[from] reqwest::Error),
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Unerwartete Antwort des Übersetzungsdienstes.")]
    InvalidResponse,
    #[error("Bitte noch {0} Sekunden warten.")]
    Cooldown(u64),
    #[error("API-Key fehlt. Bitte in den Einstellungen speichern.")]
    MissingKey,
    #[error("Sicherer Schlüsselspeicher nicht verfügbar: {0}")]
    Keyring(String),
}

#[async_trait]
pub trait TranslationProvider: Send + Sync {
    async fn translate(&self, request: &TranslationRequest) -> Result<Translation, ProviderError>;
    async fn write(&self, request: &TranslationRequest) -> Result<Translation, ProviderError>;
}

pub fn client(web: bool) -> Result<Client, ProviderError> {
    Ok(Client::builder()
        .https_only(true)
        .cookie_store(web)
        .user_agent(concat!("DeepLClient/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub fn status_message(status: u16) -> &'static str {
    match status {
        400 => "Anfrage abgelehnt. Sprache und Texteingabe prüfen.",
        401 | 403 => "Zugriff abgelehnt. API-Key prüfen oder später erneut versuchen.",
        413 => "Text zu lang.",
        429 => "Zu viele Anfragen. Bitte später erneut versuchen.",
        451 => "Dienst in dieser Region nicht verfügbar.",
        456 => "Übersetzungskontingent aufgebraucht.",
        500..=599 => "Übersetzungsdienst vorübergehend nicht verfügbar.",
        _ => "Übersetzungsdienst hat die Anfrage abgelehnt.",
    }
}

#[derive(Deserialize)]
pub struct TranslateResponse {
    pub translations: Vec<TranslatedText>,
}
#[derive(Deserialize)]
pub struct WriteResponse {
    pub improvements: Vec<TranslatedText>,
}
#[derive(Deserialize)]
pub struct TranslatedText {
    pub text: String,
    pub detected_source_language: String,
}

pub async fn decode(response: Response) -> Result<Translation, ProviderError> {
    decode_result(response, false).await
}
pub async fn decode_write(response: Response) -> Result<Translation, ProviderError> {
    decode_result(response, true).await
}
async fn decode_result(mut response: Response, write: bool) -> Result<Translation, ProviderError> {
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(ProviderError::Http {
            status,
            message: status_message(status).into(),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 1_048_576 {
            return Err(ProviderError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    let items = if write {
        serde_json::from_slice::<WriteResponse>(&bytes)
            .map_err(|_| ProviderError::InvalidResponse)?
            .improvements
    } else {
        serde_json::from_slice::<TranslateResponse>(&bytes)
            .map_err(|_| ProviderError::InvalidResponse)?
            .translations
    };
    let item = items
        .into_iter()
        .next()
        .ok_or(ProviderError::InvalidResponse)?;
    Ok(Translation {
        text: item.text,
        detected_source: item.detected_source_language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    async fn response(status: u16, body: String) -> Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        Client::new()
            .get(format!("http://{addr}"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn parses_unicode_and_rejects_empty_or_malformed_responses() {
        let r = response(
            200,
            r#"{"translations":[{"text":"Hallo 🌍","detected_source_language":"en"}]}"#.into(),
        )
        .await;
        assert_eq!(decode(r).await.unwrap().text, "Hallo 🌍");
        for body in [
            "<html>challenge</html>",
            r#"{"translations":[]}"#,
            r#"{"translations":[{"text":"missing language"}]}"#,
        ] {
            assert!(matches!(
                decode(response(200, body.into()).await).await,
                Err(ProviderError::InvalidResponse)
            ));
        }
    }

    #[tokio::test]
    async fn write_decodes_improvements_and_rejects_translation_envelope() {
        let r = response(200, r#"{"improvements":[{"text":"Das ist eine gute Idee.","detected_source_language":"de"}]}"#.into()).await;
        assert_eq!(
            decode_write(r).await.unwrap().text,
            "Das ist eine gute Idee."
        );
        for body in [
            r#"{"improvements":[]}"#,
            r#"{"translations":[{"text":"Wrong mode","detected_source_language":"de"}]}"#,
        ] {
            assert!(matches!(
                decode_write(response(200, body.into()).await).await,
                Err(ProviderError::InvalidResponse)
            ));
        }
    }

    #[tokio::test]
    async fn limits_responses_and_does_not_expose_server_secrets() {
        assert!(matches!(
            decode(response(200, "x".repeat(1_048_577)).await).await,
            Err(ProviderError::InvalidResponse)
        ));
        for status in [403, 429, 451, 456, 503] {
            let err = decode(response(status, "secret-server-token".into()).await)
                .await
                .unwrap_err();
            assert!(matches!(err, ProviderError::Http { status: s, .. } if s == status));
            assert!(!err.to_string().contains("secret-server-token"));
        }
    }
}
