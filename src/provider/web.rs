//! The installed 26.9 client uses ITA OneShot, not an assumed LMT JSON-RPC call.
//! Evidence and remaining interoperability limits: docs/REVERSE_ENGINEERING.md.
use super::{ProviderError, TranslationProvider, decode, decode_write};
use crate::core::{Translation, TranslationRequest};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct WebProvider {
    client: reqwest::Client,
    instance_id: uuid::Uuid,
    next_allowed: Mutex<Instant>,
}
impl WebProvider {
    pub fn new(instance_id: uuid::Uuid) -> Result<Self, ProviderError> {
        Ok(Self {
            client: super::client(true)?,
            instance_id,
            next_allowed: Mutex::new(Instant::now()),
        })
    }
    pub fn payload(&self, request: &TranslationRequest) -> Value {
        let mut value = json!({
            "text": [&request.text], "target_lang": Self::target_culture(&request.target), "language_model": "next-gen",
            "app_information": {"os": std::env::consts::OS, "os_version": std::env::consts::ARCH,
                "app_version": env!("CARGO_PKG_VERSION"), "app_build": env!("CARGO_PKG_VERSION"), "instance_id": self.instance_id}
        });
        if request.source != "AUTO" {
            value["source_lang"] = json!(
                request
                    .source
                    .split('-')
                    .next()
                    .unwrap_or(&request.source)
                    .to_lowercase()
            );
        }
        value
    }
    // OneShot accepts culture names, unlike the public API's language IDs.
    // Unknown tags can silently yield English with HTTP 200; keep canonical casing.
    fn target_culture(target: &str) -> String {
        match target {
            "PT" => "pt-PT".into(),
            "ZH" => "zh-Hans".into(),
            _ => match target.split_once('-') {
                Some((base, region)) => {
                    format!("{}-{}", base.to_lowercase(), region.to_uppercase())
                }
                None => target.to_lowercase(),
            },
        }
    }
    pub fn write_payload(&self, request: &TranslationRequest) -> Value {
        let language = if request.target == "EN" {
            "en-GB".into()
        } else {
            Self::target_culture(&request.target)
        };
        json!({"text": [&request.text], "requested_language": language, "style_variant": request.style.value(),
            "app_information": self.payload(request)["app_information"]})
    }
    async fn send(&self, path: &str, payload: Value) -> Result<reqwest::Response, ProviderError> {
        let mut next = self.next_allowed.lock().await;
        let now = Instant::now();
        if *next > now {
            let wait = *next - now;
            if wait > Duration::from_secs(1) {
                return Err(ProviderError::Cooldown(wait.as_secs() + 1));
            }
            tokio::time::sleep(wait).await;
        }
        *next = Instant::now() + Duration::from_secs(1);
        let response = self
            .client
            .post(format!("https://oneshot-free.www.deepl.com/v1/{path}"))
            .header("Referer", "https://www.deepl.com/")
            .json(&payload)
            .send()
            .await?;
        if [403, 429].contains(&response.status().as_u16()) {
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(1, 3600);
            *next = Instant::now() + Duration::from_secs(delay);
        }
        Ok(response)
    }
}
#[async_trait]
impl TranslationProvider for WebProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<Translation, ProviderError> {
        // Serialize a session. Never rotate identity to get around a server refusal.
        decode(self.send("translate", self.payload(request)).await?).await
    }
    async fn write(&self, request: &TranslationRequest) -> Result<Translation, ProviderError> {
        decode_write(self.send("write", self.write_payload(request)).await?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Sends four synthetic translation requests and one Chinese Write request"]
    async fn live_language_routes_smoke() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let mut records = Vec::new();
        let mut request = TranslationRequest {
            text: "Das ist ein schönes Haus. Ich fahre mit dem Zug zur Arbeit.".into(),
            source: "DE".into(),
            target: "PT".into(),
            style: Default::default(),
        };
        for (target, marker) in [
            ("PT", "comboio"),
            ("PT-BR", "trem"),
            ("PT-PT", "comboio"),
            ("ZH", "火车"),
        ] {
            request.target = target.into();
            let result = provider.translate(&request).await.unwrap();
            records.push(json!({"payload": provider.payload(&request), "text": result.text, "detected_source": result.detected_source}));
            assert!(result.text.contains(marker), "{target}: {}", result.text);
        }
        request.text = "这是一个很好的主意。".into();
        request.source = "AUTO".into();
        let result = provider.write(&request).await.unwrap();
        records.push(json!({"payload": provider.write_payload(&request), "text": result.text, "detected_source": result.detected_source}));
        std::fs::create_dir_all(".tools/language-audit").unwrap();
        std::fs::write(
            ".tools/language-audit/fixed-live.json",
            serde_json::to_string_pretty(&records).unwrap(),
        )
        .unwrap();
        assert!(
            result
                .text
                .chars()
                .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "Chinese Write: {}",
            result.text
        );
    }

    #[test]
    fn translation_routes_use_canonical_oneshot_cultures_instead_of_api_ids() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let mut request = TranslationRequest {
            text: "Das ist ein schönes Haus.".into(),
            source: "DE".into(),
            target: "PT".into(),
            style: Default::default(),
        };
        for (target, expected) in [
            ("PT", "pt-PT"),
            ("PT-BR", "pt-BR"),
            ("PT-PT", "pt-PT"),
            ("ZH", "zh-Hans"),
            ("EN-GB", "en-GB"),
            ("EN-US", "en-US"),
            ("DE", "de"),
            ("AR", "ar"),
        ] {
            request.target = target.into();
            assert_eq!(
                provider.payload(&request)["target_lang"],
                expected,
                "{target}"
            );
        }
    }

    #[test]
    fn write_uses_simplified_chinese_culture() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let request = TranslationRequest {
            text: "这是一个测试。".into(),
            source: "AUTO".into(),
            target: "ZH".into(),
            style: Default::default(),
        };
        assert_eq!(
            provider.write_payload(&request)["requested_language"],
            "zh-Hans"
        );
    }

    #[test]
    fn four_styles_use_observed_web_values_and_official_api_field() {
        use crate::core::WritingStyle;
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let mut request: TranslationRequest =
            serde_json::from_str(r#"{"text":"Test","source":"AUTO","target":"DE"}"#).unwrap();
        assert_eq!(request.style, WritingStyle::Default);
        assert!(
            crate::provider::api::ApiProvider::write_payload(&request)
                .get("writing_style")
                .is_none()
        );
        for (style, value) in [
            (WritingStyle::Simple, "simple"),
            (WritingStyle::Business, "business"),
            (WritingStyle::Academic, "academic"),
            (WritingStyle::Casual, "casual"),
        ] {
            request.style = style;
            assert_eq!(provider.write_payload(&request)["style_variant"], value);
            assert_eq!(
                crate::provider::api::ApiProvider::write_payload(&request)["writing_style"],
                value
            );
            assert!(request.validate_write().is_ok());
        }
        request.target = "JA".into();
        assert!(request.validate_write().is_err());
        request.style = WritingStyle::Default;
        assert!(request.validate_write().is_ok());
        assert!(
            serde_json::from_str::<TranslationRequest>(
                r#"{"text":"Test","source":"AUTO","target":"DE","style":"made-up"}"#
            )
            .is_err()
        );
    }
    #[tokio::test]
    #[ignore = "Sends four synthetic style requests to DeepL"]
    async fn live_write_styles_smoke() {
        use crate::core::WritingStyle;
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        for style in [
            WritingStyle::Simple,
            WritingStyle::Business,
            WritingStyle::Academic,
            WritingStyle::Casual,
        ] {
            let request=TranslationRequest{text:"Ich wollte Ihnen kurz sagen, dass wir das Treffen leider verschieben müssen, weil noch einige wichtige Informationen fehlen.".into(),source:"AUTO".into(),target:"DE".into(),style};
            let result = provider.write(&request).await.unwrap();
            assert!(!result.text.is_empty());
            assert_eq!(result.detected_source.to_uppercase(), "DE");
            println!("{}: {}", style.value().unwrap(), result.text);
        }
    }
    #[tokio::test]
    #[ignore = "Sends a short synthetic Write test to the real DeepL service"]
    async fn live_write_smoke() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let request = TranslationRequest {
            text: "Das ist ein gute Idee.".into(),
            source: "AUTO".into(),
            target: "DE".into(),
            style: Default::default(),
        };
        let result = provider.write(&request).await.unwrap();
        assert!(!result.text.is_empty());
        assert_eq!(result.detected_source.to_uppercase(), "DE");
        println!("Write: {} -> {}", request.text, result.text);
    }
    #[test]
    fn write_preserves_language_variants_and_uses_separate_protocol() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let mut request = TranslationRequest {
            text: "This are a test".into(),
            source: "AUTO".into(),
            target: "EN".into(),
            style: Default::default(),
        };
        let p = provider.write_payload(&request);
        assert_eq!(p["requested_language"], "en-GB");
        assert!(p.get("target_lang").is_none());
        assert!(p.get("language_model").is_none());
        assert_eq!(p["style_variant"], Value::Null);
        request.target = "EN-US".into();
        assert_eq!(
            provider.write_payload(&request)["requested_language"],
            "en-US"
        );
        request.target = "NL".into();
        assert!(request.validate_write().is_err());
    }
    #[tokio::test]
    #[ignore = "Sends a short synthetic test text to the real DeepL service"]
    async fn live_oneshot_smoke() {
        let provider = WebProvider::new(uuid::Uuid::new_v4()).unwrap();
        let request = TranslationRequest {
            text: "Hello.".into(),
            source: "AUTO".into(),
            target: "DE".into(),
            style: Default::default(),
        };
        let result = provider.translate(&request).await.unwrap();
        assert!(!result.text.is_empty());
        assert_eq!(result.detected_source.to_uppercase(), "EN");
        println!(
            "OneShot: {} -> {} ({})",
            request.text, result.text, result.detected_source
        );
    }
    #[test]
    fn oneshot_uses_observed_dto_and_stable_instance() {
        let id = uuid::Uuid::new_v4();
        let provider = WebProvider::new(id).unwrap();
        let mut req = TranslationRequest {
            text: "Hello 🌍".into(),
            source: "AUTO".into(),
            target: "DE".into(),
            style: Default::default(),
        };
        let p = provider.payload(&req);
        assert_eq!(p["text"][0], req.text);
        assert_eq!(p["language_model"], "next-gen");
        assert!(p.get("source_lang").is_none());
        assert!(p.get("method").is_none());
        assert_eq!(p["app_information"]["instance_id"], id.to_string());
        req.source = "EN-US".into();
        assert_eq!(provider.payload(&req)["source_lang"], "en");
    }
}
