use super::{ProviderError, TranslationProvider, decode, decode_write};
use crate::core::{
    Translation, TranslationRequest,
    config::{ProviderMode, key_entry},
};
use async_trait::async_trait;
use serde_json::json;

pub struct ApiProvider {
    pub client: reqwest::Client,
    pub pro: bool,
}
impl ApiProvider {
    pub fn write_payload(request: &TranslationRequest) -> serde_json::Value {
        let mut payload =
            json!({"text": [&request.text], "target_lang": request.target.to_lowercase()});
        if let Some(style) = request.style.value() {
            payload["writing_style"] = json!(style);
        }
        payload
    }
    pub fn new(mode: ProviderMode) -> Result<Self, ProviderError> {
        Ok(Self {
            client: super::client(false)?,
            pro: mode == ProviderMode::ApiPro,
        })
    }
    async fn key(&self) -> Result<String, ProviderError> {
        let key = tokio::task::spawn_blocking(|| {
            key_entry()
                .map_err(|e| ProviderError::Keyring(e.to_string()))?
                .get_password()
                .map_err(|e| match e {
                    keyring::Error::NoEntry => ProviderError::MissingKey,
                    other => ProviderError::Keyring(other.to_string()),
                })
        })
        .await
        .map_err(|_| ProviderError::MissingKey)??;
        if key.trim().is_empty() {
            return Err(ProviderError::MissingKey);
        }
        Ok(key)
    }
}
#[async_trait]
impl TranslationProvider for ApiProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<Translation, ProviderError> {
        let key = self.key().await?;
        let url = if self.pro {
            "https://api.deepl.com/v2/translate"
        } else {
            "https://api-free.deepl.com/v2/translate"
        };
        let mut payload = json!({"text": [&request.text], "target_lang": &request.target});
        if request.source != "AUTO" {
            payload["source_lang"] =
                json!(request.source.split('-').next().unwrap_or(&request.source));
        }
        decode(
            self.client
                .post(url)
                .header("Authorization", format!("DeepL-Auth-Key {}", key.trim()))
                .json(&payload)
                .send()
                .await?,
        )
        .await
    }
    async fn write(&self, request: &TranslationRequest) -> Result<Translation, ProviderError> {
        let key = self.key().await?;
        let host = if self.pro {
            "api.deepl.com"
        } else {
            "api-free.deepl.com"
        };
        let payload = Self::write_payload(request);
        decode_write(
            self.client
                .post(format!("https://{host}/v2/write/rephrase"))
                .header("Authorization", format!("DeepL-Auth-Key {}", key.trim()))
                .json(&payload)
                .send()
                .await?,
        )
        .await
    }
}
