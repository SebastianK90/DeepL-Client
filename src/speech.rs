//! On-demand Edge Read Aloud. No user text or generated audio is persisted.
use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use msedge_tts::tts::{
    SpeechConfig,
    stream::{SynthesizedResponse, tokio_runtime::msedge_tts_split_async},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MAX_TEXT_CHARS: usize = 5_000;
const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const VOICE_FAILURE_TTL: Duration = Duration::from_secs(10 * 60);
const VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";

#[derive(Clone, Deserialize)]
struct Voice {
    #[serde(rename = "ShortName")]
    short_name: String,
    #[serde(rename = "Locale")]
    locale: String,
    #[serde(rename = "Gender")]
    gender: String,
}

#[derive(Clone, Serialize)]
pub struct SpeechOutput {
    pub audio_base64: String,
    pub voice: String,
    pub preferred_voice: String,
    pub locale: String,
    pub fallback: bool,
}

#[derive(Serialize)]
#[serde(tag = "event", content = "data", rename_all = "lowercase")]
pub enum SpeechEvent {
    Started(SpeechOutput),
    Audio(String),
    Finished,
}

struct ResolvedVoice {
    voice: String,
    preferred_voice: String,
    locale: String,
    fallback: bool,
}

pub struct SpeechClient {
    http: reqwest::Client,
    catalog: Mutex<Option<(Instant, Vec<Voice>)>>,
    failed_voices: Mutex<HashMap<String, Instant>>,
}

impl SpeechClient {
    pub fn new() -> Result<Self> {
        // Both the existing HTTP client and upstream TTS pull in rustls providers.
        // Install one explicitly to avoid ambiguous-provider panics.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Mozilla/5.0 Edg/155.0.0.0")
                .build()
                .map_err(|_| anyhow::anyhow!("Sprachausgabe konnte nicht initialisiert werden."))?,
            catalog: Mutex::new(None),
            failed_voices: Mutex::new(HashMap::new()),
        })
    }

    /// Dropping this future drops the socket; there are no detached tasks.
    #[cfg(test)]
    pub async fn synthesize(&self, text: &str, language: &str) -> Result<SpeechOutput> {
        self.synthesize_stream(text, language, None).await
    }

    pub async fn synthesize_stream(
        &self,
        text: &str,
        language: &str,
        sink: Option<&(dyn Fn(SpeechEvent) -> Result<()> + Send + Sync)>,
    ) -> Result<SpeechOutput> {
        let escaped = prepare_text(text)?;
        preference(language)?;
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let selection = async {
                let mut cache = self.catalog.lock().await;
                if cache
                    .as_ref()
                    .is_none_or(|(at, _)| at.elapsed() >= CATALOG_TTL)
                {
                    let catalog = self.fetch_catalog().await?;
                    *cache = Some((Instant::now(), catalog));
                }
                let catalog = &cache.as_ref().expect("catalog initialized").1;
                Ok::<_, anyhow::Error>((
                    resolve_voice(language, catalog)?,
                    multilingual_recovery(language, catalog),
                ))
            };
            let connection = async {
                msedge_tts_split_async().await.map_err(|_| {
                    anyhow::anyhow!("Edge-Sprachausgabe ist derzeit nicht erreichbar.")
                })
            };
            let ((mut selected, mut recovery), initial_connection) =
                tokio::try_join!(selection, connection)?;
            let native_voice = selected.voice.clone();
            if recovery.is_some()
                && self
                    .failed_voices
                    .lock()
                    .await
                    .get(&native_voice)
                    .is_some_and(|at| at.elapsed() < VOICE_FAILURE_TTL)
            {
                selected = recovery.take().expect("recovery checked");
            }
            let mut output = SpeechOutput {
                audio_base64: String::new(),
                voice: selected.voice.clone(),
                preferred_voice: selected.preferred_voice,
                locale: selected.locale,
                fallback: selected.fallback,
            };
            let mut config = SpeechConfig {
                voice_name: selected.voice.clone(),
                audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
                pitch: 0,
                rate: 0,
                volume: 0,
            };
            let mut audio = Vec::new();
            let mut connection = Some(initial_connection);
            // Edge sometimes resets a voice backend before sending any audio.
            // Retry only before playback could start; never repeat audible text.
            for attempt in 0..3 {
                if attempt > 0
                    && let Some(alternative) = recovery.take()
                {
                    self.failed_voices
                        .lock()
                        .await
                        .insert(native_voice.clone(), Instant::now());
                    config.voice_name = alternative.voice.clone();
                    output.voice = alternative.voice;
                    output.fallback = true;
                }
                if let Some(send) = sink {
                    send(SpeechEvent::Started(output.clone()))?;
                }
                let (mut sender, mut receiver) = match connection.take() {
                    Some(value) => value,
                    None => msedge_tts_split_async().await.map_err(|_| {
                        anyhow::anyhow!("Edge-Sprachausgabe ist derzeit nicht erreichbar.")
                    })?,
                };
                if sender.send(&escaped, &config).await.is_err() {
                    if attempt < 2 {
                        continue;
                    }
                    bail!("Edge-Sprachausgabe konnte nicht gestartet werden.");
                }
                let mut receive_failed = false;
                while receiver.can_read().await {
                    match tokio::time::timeout(Duration::from_secs(12), receiver.read()).await {
                        Ok(Ok(Some(SynthesizedResponse::AudioBytes(chunk)))) => {
                            append_audio(&mut audio, &chunk)?;
                            if !chunk.is_empty()
                                && let Some(send) = sink
                            {
                                send(SpeechEvent::Audio(STANDARD.encode(&chunk)))?;
                            }
                        }
                        Ok(Ok(Some(SynthesizedResponse::AudioMetadata(_)))) | Ok(Ok(None)) => {
                            tokio::task::yield_now().await;
                        }
                        Ok(Err(_)) | Err(_) => {
                            receive_failed = true;
                            break;
                        }
                    }
                }
                if !audio.is_empty() {
                    if receive_failed {
                        bail!("Edge-Sprachausgabe konnte nicht empfangen werden.");
                    }
                    break;
                }
                if attempt == 2 {
                    bail!("Edge-Sprachausgabe hat keine Audiodaten geliefert.");
                }
            }
            if sink.is_none() {
                output.audio_base64 = STANDARD.encode(audio);
            } else if let Some(send) = sink {
                send(SpeechEvent::Finished)?;
            }
            Ok(output)
        })
        .await
        .map_err(|_| anyhow::anyhow!("Zeitüberschreitung bei der Edge-Sprachausgabe."))?
    }

    async fn fetch_catalog(&self) -> Result<Vec<Voice>> {
        let mut response = self
            .http
            .get(VOICES_URL)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| anyhow::anyhow!("Edge-Stimmenliste konnte nicht geladen werden."))?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_CATALOG_BYTES as u64)
        {
            bail!("Edge-Stimmenliste überschreitet die Größenbegrenzung.");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Edge-Stimmenliste konnte nicht empfangen werden."))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
                bail!("Edge-Stimmenliste überschreitet die Größenbegrenzung.");
            }
            bytes.extend_from_slice(&chunk);
        }
        let voices: Vec<Voice> = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Edge-Stimmenliste ist ungültig."))?;
        if voices.is_empty() {
            bail!("Edge-Stimmenliste ist leer.");
        }
        Ok(voices)
    }
}

fn append_audio(audio: &mut Vec<u8>, chunk: &[u8]) -> Result<()> {
    if audio.len().saturating_add(chunk.len()) > MAX_AUDIO_BYTES {
        bail!("Die Sprachausgabe überschreitet die Größenbegrenzung.");
    }
    audio.extend_from_slice(chunk);
    Ok(())
}

fn prepare_text(text: &str) -> Result<String> {
    if text.trim().is_empty() {
        bail!("Kein Text für die Sprachausgabe vorhanden.");
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        bail!("Die Sprachausgabe unterstützt höchstens 5000 Zeichen.");
    }
    if text.chars().any(|c| {
        !matches!(c, '\t' | '\n' | '\r') && (c < ' ' || c == '\u{fffe}' || c == '\u{ffff}')
    }) {
        bail!("Der Text enthält ungültige Steuerzeichen.");
    }
    Ok(text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

/// locale, requested voice, preferred same-locale substitute (when requested is absent).
fn preference(language: &str) -> Result<(&'static str, &'static str, &'static str)> {
    Ok(
        match language
            .trim()
            .to_ascii_uppercase()
            .replace('_', "-")
            .as_str()
        {
            "DE" | "DE-DE" => ("de-DE", "de-DE-ConradNeural", ""),
            "EN" | "EN-GB" => ("en-GB", "en-GB-RyanNeural", ""),
            "EN-US" => ("en-US", "en-US-BrianNeural", ""),
            "FR" | "FR-FR" => (
                "fr-FR",
                "fr-FR-Remy:DragonHDLatestNeural",
                "fr-FR-RemyMultilingualNeural",
            ),
            "ES" | "ES-ES" => (
                "es-ES",
                "es-ES-Tristan:DragonHDLatestNeural",
                "es-ES-AlvaroNeural",
            ),
            "IT" | "IT-IT" => (
                "it-IT",
                "it-IT-Alessio:DragonHDLatestNeural",
                "it-IT-DiegoNeural",
            ),
            "NL" | "NL-NL" => ("nl-NL", "nl-NL-MaartenNeural", ""),
            "PL" | "PL-PL" => ("pl-PL", "pl-PL-MarekNeural", ""),
            "PT" | "PT-PT" => ("pt-PT", "pt-PT-DuarteNeural", ""),
            "PT-BR" => (
                "pt-BR",
                "pt-BR-Macerio:DragonHDLatestNeural",
                "pt-BR-AntonioNeural",
            ),
            "JA" | "JA-JP" => (
                "ja-JP",
                "ja-JP-Masaru:DragonHDLatestNeural",
                "ja-JP-KeitaNeural",
            ),
            "ZH" | "ZH-HANS" | "ZH-HANT" | "ZH-CN" => (
                "zh-CN",
                "zh-CN-Yunxiao:DragonHDFlashLatestNeural",
                "zh-CN-YunxiNeural",
            ),
            "KO" | "KO-KR" => (
                "ko-KR",
                "ko-KR-Hyunsu:DragonHDLatestNeural",
                "ko-KR-HyunsuMultilingualNeural",
            ),
            "RU" | "RU-RU" => ("ru-RU", "ru-RU-DmitryNeural", ""),
            "UK" | "UK-UA" => ("uk-UA", "uk-UA-OstapNeural", ""),
            "CS" | "CZ" | "CS-CZ" | "CZ-CZ" => ("cs-CZ", "cs-CZ-AntoninNeural", ""),
            "DA" | "DA-DK" => ("da-DK", "da-DK-JeppeNeural", ""),
            "EL" | "EL-GR" => ("el-GR", "el-GR-NestorasNeural", ""),
            "FI" | "FI-FI" => ("fi-FI", "fi-FI-HarriNeural", ""),
            "HU" | "HU-HU" => ("hu-HU", "hu-HU-TamasNeural", ""),
            "ID" | "ID-ID" => ("id-ID", "id-ID-ArdiNeural", ""),
            "NB" | "NB-NO" => ("nb-NO", "nb-NO-FinnNeural", ""),
            "RO" | "RO-RO" => ("ro-RO", "ro-RO-EmilNeural", ""),
            "SK" | "SK-SK" => ("sk-SK", "sk-SK-LukasNeural", ""),
            "SL" | "SL-SI" => ("sl-SI", "sl-SI-RokNeural", ""),
            "TR" | "TR-TR" => ("tr-TR", "tr-TR-AhmetNeural", ""),
            "BG" | "BG-BG" => ("bg-BG", "bg-BG-BorislavNeural", ""),
            "ET" | "ET-EE" => ("et-EE", "et-EE-KertNeural", ""),
            "LT" | "LT-LT" => ("lt-LT", "lt-LT-LeonasNeural", ""),
            "LV" | "LV-LV" => ("lv-LV", "lv-LV-NilsNeural", ""),
            "AR" | "AR-AE" => ("ar-AE", "ar-AE-HamdanNeural", ""),
            "SV" | "SV-SE" => ("sv-SE", "sv-SE-MattiasNeural", ""),
            _ => bail!("Für diese Sprache ist keine Edge-Stimme konfiguriert."),
        },
    )
}

fn resolve_voice(language: &str, catalog: &[Voice]) -> Result<ResolvedVoice> {
    let (locale, preferred, substitute) = preference(language)?;
    let valid = |v: &&Voice| {
        v.locale.eq_ignore_ascii_case(locale)
            && v.gender.eq_ignore_ascii_case("Male")
            && v.short_name.starts_with(&format!("{locale}-"))
            && v.short_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b':')
    };
    let selected = catalog
        .iter()
        .filter(valid)
        .find(|v| v.short_name == preferred)
        .or_else(|| {
            catalog
                .iter()
                .filter(valid)
                .find(|v| v.short_name == substitute)
        })
        .or_else(|| catalog.iter().filter(valid).min_by_key(|v| &v.short_name))
        .ok_or_else(|| anyhow::anyhow!("Keine männliche Edge-Stimme für {locale} verfügbar."))?;
    Ok(ResolvedVoice {
        voice: selected.short_name.clone(),
        preferred_voice: preferred.into(),
        locale: locale.into(),
        fallback: selected.short_name != preferred,
    })
}

// Explicit exception to same-locale voices: this male multilingual model supports
// Czech, Turkish, Swedish and Russian (Microsoft SSML multilingual voice table).
// Require its exact audited catalog identity/gender.
fn multilingual_recovery(language: &str, catalog: &[Voice]) -> Option<ResolvedVoice> {
    let (locale, preferred, _) = preference(language).ok()?;
    if !matches!(locale, "cs-CZ" | "tr-TR" | "sv-SE" | "ru-RU") {
        return None;
    }
    let voice = catalog.iter().find(|v| {
        v.short_name == "en-US-BrianMultilingualNeural"
            && v.locale == "en-US"
            && v.gender.eq_ignore_ascii_case("Male")
    })?;
    Some(ResolvedVoice {
        voice: voice.short_name.clone(),
        preferred_voice: preferred.into(),
        locale: locale.into(),
        fallback: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "live Russian backend investigation using fixed synthetic text only"]
    async fn live_russian_backend_probe() {
        std::fs::create_dir_all(".tools/russian-tts").unwrap();
        let client = SpeechClient::new().unwrap();
        let catalog = client.fetch_catalog().await.unwrap();
        let identities: Vec<_> = catalog
            .iter()
            .filter(|v| {
                matches!(
                    v.short_name.as_str(),
                    "ru-RU-DmitryNeural" | "en-US-BrianMultilingualNeural"
                )
            })
            .map(|v| serde_json::json!({"voice":v.short_name,"locale":v.locale,"gender":v.gender}))
            .collect();
        let short = "Здравствуйте. Это короткая проверка русского голоса.";
        let long = "Это синтетическая проверка русской речи. Мы проверяем начало воспроизведения и получение полного звукового ответа. ".repeat(10);
        let mut records = Vec::new();
        for repeat in 0..2 {
            for voice in ["ru-RU-DmitryNeural", "en-US-BrianMultilingualNeural"] {
                for (sample, text) in [("short", short), ("long", long.as_str())] {
                    let start = Instant::now();
                    let mut audio = Vec::new();
                    let mut first = None;
                    let run = async {
                        let (mut sender, mut receiver) = msedge_tts_split_async().await?;
                        sender
                            .send(
                                &prepare_text(text)?,
                                &SpeechConfig {
                                    voice_name: voice.into(),
                                    audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
                                    pitch: 0,
                                    rate: 0,
                                    volume: 0,
                                },
                            )
                            .await?;
                        while receiver.can_read().await {
                            if let Some(SynthesizedResponse::AudioBytes(chunk)) =
                                receiver.read().await?
                            {
                                if !chunk.is_empty() {
                                    first.get_or_insert(start.elapsed().as_millis());
                                }
                                append_audio(&mut audio, &chunk)?;
                            }
                        }
                        Ok::<_, anyhow::Error>(())
                    };
                    let result = tokio::time::timeout(Duration::from_secs(18), run).await;
                    let error = match result {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(_) => Some("18s probe timeout".into()),
                    };
                    let record = serde_json::json!({"repeat":repeat,"voice":voice,"sample":sample,"audio_bytes":audio.len(),"first_audio_ms":first,"complete_ms":start.elapsed().as_millis(),"error":error});
                    eprintln!("{record}");
                    if !audio.is_empty() {
                        std::fs::write(
                            format!(".tools/russian-tts/{voice}-{sample}-{repeat}.mp3"),
                            &audio,
                        )
                        .unwrap();
                    }
                    records.push(record);
                    std::fs::write(
                        ".tools/russian-tts/raw-backends.json",
                        serde_json::to_vec_pretty(
                            &serde_json::json!({"catalog":identities,"samples":records}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }

    #[tokio::test]
    #[ignore = "live Russian application streaming and buffered synthesis; synthetic text only"]
    async fn live_russian_application_recovery() {
        std::fs::create_dir_all(".tools/russian-tts").unwrap();
        let client = SpeechClient::new().unwrap();
        let long = "Это синтетическая проверка русской речи. Мы проверяем начало воспроизведения и получение полного звукового ответа. ".repeat(3);
        let mut records = Vec::new();
        for repeat in 0..2 {
            for (sample, text) in [
                (
                    "short",
                    "Здравствуйте. Это короткая проверка русского голоса.",
                ),
                ("long", long.as_str()),
            ] {
                let start = Instant::now();
                let captured = std::sync::Mutex::new((None, Vec::new(), false, Vec::new()));
                let send = |event| {
                    let mut captured = captured.lock().unwrap();
                    match event {
                        SpeechEvent::Started(output) => captured.3.push(output.voice),
                        SpeechEvent::Audio(data) => {
                            captured.0.get_or_insert(start.elapsed().as_millis());
                            captured.1.extend(STANDARD.decode(data).unwrap());
                        }
                        SpeechEvent::Finished => captured.2 = true,
                    }
                    Ok(())
                };
                let streaming = repeat == 0;
                let result = tokio::time::timeout(
                    Duration::from_secs(45),
                    client.synthesize_stream(
                        text,
                        "RU",
                        if streaming { Some(&send) } else { None },
                    ),
                )
                .await;
                let (first, mut audio, finished, started) = captured.into_inner().unwrap();
                let mut record = serde_json::json!({"repeat":repeat,"sample":sample,"streaming":streaming,"first_audio_ms":first,"complete_ms":start.elapsed().as_millis(),"finished":finished,"started_voices":started});
                match result {
                    Ok(Ok(output)) => {
                        if !streaming {
                            audio = STANDARD.decode(&output.audio_base64).unwrap();
                        }
                        record["voice"] = output.voice.into();
                        record["fallback"] = output.fallback.into();
                        record["preferred_voice"] = output.preferred_voice.into();
                        record["locale"] = output.locale.into();
                        record["passed"] = (audio.len() > 1000 && (!streaming || finished)).into();
                    }
                    Ok(Err(error)) => {
                        record["error"] = error.to_string().into();
                        record["passed"] = false.into();
                    }
                    Err(_) => {
                        record["error"] = "45s application probe timeout".into();
                        record["passed"] = false.into();
                    }
                }
                record["audio_bytes"] = audio.len().into();
                eprintln!("{record}");
                if !audio.is_empty() {
                    std::fs::write(
                        format!(".tools/russian-tts/application-{sample}-{repeat}.mp3"),
                        audio,
                    )
                    .unwrap();
                }
                records.push(record);
                std::fs::write(
                    ".tools/russian-tts/application.json",
                    serde_json::to_vec_pretty(&records).unwrap(),
                )
                .unwrap();
            }
        }
        assert!(records.iter().all(|r| r["passed"] == true));
    }

    #[test]
    fn multilingual_recovery_is_explicit_male_and_language_bounded() {
        let catalog = vec![Voice {
            short_name: "en-US-BrianMultilingualNeural".into(),
            locale: "en-US".into(),
            gender: "Male".into(),
        }];
        for language in ["CS", "CZ", "TR", "SV", "RU", "RU-RU"] {
            let voice = multilingual_recovery(language, &catalog).unwrap();
            assert_eq!(voice.voice, "en-US-BrianMultilingualNeural");
            assert!(voice.fallback);
            assert_eq!(voice.locale, preference(language).unwrap().0);
        }
        assert!(multilingual_recovery("DE", &catalog).is_none());
        assert!(multilingual_recovery("JA", &catalog).is_none());
        let female = vec![Voice {
            short_name: "en-US-BrianMultilingualNeural".into(),
            locale: "en-US".into(),
            gender: "Female".into(),
        }];
        assert!(multilingual_recovery("CS", &female).is_none());
        assert!(multilingual_recovery("RU", &female).is_none());
        let wrong_locale = vec![Voice {
            short_name: "en-US-BrianMultilingualNeural".into(),
            locale: "ru-RU".into(),
            gender: "Male".into(),
        }];
        assert!(multilingual_recovery("RU", &wrong_locale).is_none());
    }
    #[tokio::test]
    #[ignore = "live male multilingual alternatives, synthetic text only"]
    async fn live_male_alternatives() {
        let _client = SpeechClient::new().unwrap();
        for (language, text) in [
            ("CS", "Dobrý den. Toto je zkouška českého hlasu."),
            ("TR", "Merhaba. Bu Türkçe ses için bir denemedir."),
            ("SV", "Hej. Detta är ett test av den svenska rösten."),
        ] {
            let start = Instant::now();
            let (mut sender, mut receiver) = msedge_tts_split_async().await.unwrap();
            sender
                .send(
                    &prepare_text(text).unwrap(),
                    &SpeechConfig {
                        voice_name: "en-US-BrianMultilingualNeural".into(),
                        audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
                        pitch: 0,
                        rate: 0,
                        volume: 0,
                    },
                )
                .await
                .unwrap();
            let mut audio = Vec::new();
            let mut first = None;
            let work = async {
                while receiver.can_read().await {
                    if let Some(SynthesizedResponse::AudioBytes(bytes)) =
                        receiver.read().await.unwrap()
                    {
                        first.get_or_insert(start.elapsed().as_millis());
                        audio.extend(bytes);
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(15), work)
                .await
                .unwrap();
            eprintln!(
                "{language} Brian: {}bytes first={first:?}ms total={}ms",
                audio.len(),
                start.elapsed().as_millis()
            );
            assert!(audio.len() > 1000);
            std::fs::write(
                format!(".tools/edge-brian-{}.mp3", language.to_lowercase()),
                audio,
            )
            .unwrap();
        }
    }
    #[tokio::test]
    #[ignore = "real Edge streaming with fixed synthetic text; records timing and MP3 fixtures"]
    async fn live_reported_languages() {
        let client = SpeechClient::new().unwrap();
        let mut evidence = Vec::new();
        let long="Dies ist ein synthetischer Test der direkten Sprachausgabe. Die Wiedergabe beginnt bereits während weitere Audiodaten eintreffen. ".repeat(8);
        for (language, text, name) in [
            ("CS", "Dobrý den. Toto je zkouška českého hlasu.", "cs"),
            ("SV", "Hej. Detta är ett test av den svenska rösten.", "sv"),
            ("TR", "Merhaba. Bu Türkçe ses için bir denemedir.", "tr"),
            ("CZ", "Dobrý den.", "cz-alias"),
            ("DE", long.as_str(), "de-long"),
        ] {
            let start = Instant::now();
            let captured = std::sync::Mutex::new((None, Vec::new(), false));
            let send = |event| {
                let mut captured = captured.lock().unwrap();
                match event {
                    SpeechEvent::Audio(data) => {
                        captured.0.get_or_insert(start.elapsed().as_millis());
                        captured.1.extend(STANDARD.decode(data).unwrap());
                    }
                    SpeechEvent::Finished => captured.2 = true,
                    SpeechEvent::Started(_) => {}
                }
                Ok(())
            };
            let result = client.synthesize_stream(text, language, Some(&send)).await;
            let (first, audio, finished) = captured.into_inner().unwrap();
            let elapsed = start.elapsed().as_millis();
            let mut record = serde_json::json!({"language":language,"sample":name,"first_audio_ms":first,"complete_ms":elapsed,"audio_bytes":audio.len(),"finished":finished});
            match result {
                Ok(output) => {
                    assert!(output.audio_base64.is_empty());
                    assert!(audio.len() > 1000 && finished);
                    assert!(first.unwrap() < elapsed);
                    record["voice"] = output.voice.into();
                    record["result"] = "passed".into();
                    std::fs::write(format!(".tools/edge-stream-{name}.mp3"), audio).unwrap();
                }
                Err(error) => {
                    record["result"] = "failed".into();
                    record["error"] = error.to_string().into();
                }
            }
            eprintln!("{record}");
            evidence.push(record);
        }
        std::fs::write(
            "docs/evidence/edge-stream-smoke.json",
            serde_json::to_vec_pretty(&serde_json::json!({"date":"2026-10-28","samples":evidence}))
                .unwrap(),
        )
        .unwrap();
        assert!(evidence.iter().all(|record| record["result"] == "passed"));
    }

    #[test]
    fn ukrainian_is_not_british_english() {
        assert_eq!(preference("UK").unwrap().0, "uk-UA");
        assert_eq!(preference("EN-GB").unwrap().0, "en-GB");
        assert!(preference("xx").is_err());
    }
    #[test]
    fn text_is_xml_escaped_and_bounded() {
        assert_eq!(
            prepare_text("A & <B> \" '").unwrap(),
            "A &amp; &lt;B&gt; &quot; &apos;"
        );
        assert!(prepare_text(" \n ").is_err());
        assert!(prepare_text("a\u{0}b").is_err());
        assert!(prepare_text(&"a".repeat(5001)).is_err());
        assert!(prepare_text(&"界".repeat(5000)).is_ok());
    }
    #[test]
    fn never_choose_a_different_locale_or_female_voice() {
        let catalog = vec![
            Voice {
                short_name: "fr-CA-JeanNeural".into(),
                locale: "fr-CA".into(),
                gender: "Male".into(),
            },
            Voice {
                short_name: "fr-FR-DeniseNeural".into(),
                locale: "fr-FR".into(),
                gender: "Female".into(),
            },
        ];
        assert!(resolve_voice("FR", &catalog).is_err());
    }
    #[test]
    fn preferred_voice_wins_and_hd_fallback_is_reported() {
        let catalog: Vec<Voice> =
            serde_json::from_str(include_str!("../assets/speech-voices.json")).unwrap();
        let de = resolve_voice("DE", &catalog).unwrap();
        assert_eq!(de.voice, "de-DE-ConradNeural");
        assert!(!de.fallback);
        let fr = resolve_voice("FR", &catalog).unwrap();
        assert_eq!(fr.preferred_voice, "fr-FR-Remy:DragonHDLatestNeural");
        assert_eq!(fr.voice, "fr-FR-RemyMultilingualNeural");
        assert!(fr.fallback);
    }
    #[test]
    fn all_supported_languages_have_a_male_voice_in_audited_catalog() {
        let catalog: Vec<Voice> =
            serde_json::from_str(include_str!("../assets/speech-voices.json")).unwrap();
        for language in [
            "DE", "EN", "EN-GB", "EN-US", "FR", "ES", "IT", "NL", "PL", "PT", "PT-PT", "PT-BR",
            "JA", "ZH", "ZH-HANS", "ZH-HANT", "KO", "RU", "UK", "CS", "DA", "EL", "FI", "HU", "ID",
            "NB", "RO", "SK", "SL", "TR", "BG", "ET", "LT", "LV", "AR", "SV",
        ] {
            let selected = resolve_voice(language, &catalog)
                .unwrap_or_else(|error| panic!("{language}: {error}"));
            assert_eq!(selected.locale, preference(language).unwrap().0);
        }
    }

    #[test]
    fn audio_limit_is_checked_before_appending() {
        let mut audio = vec![0; MAX_AUDIO_BYTES - 1];
        append_audio(&mut audio, &[1]).unwrap();
        assert!(append_audio(&mut audio, &[2]).is_err());
        assert_eq!(audio.len(), MAX_AUDIO_BYTES);
    }

    #[test]
    fn exact_hd_voice_is_selected_if_service_adds_it() {
        let catalog = vec![Voice {
            short_name: "fr-FR-Remy:DragonHDLatestNeural".into(),
            locale: "fr-FR".into(),
            gender: "Male".into(),
        }];
        let selected = resolve_voice("FR", &catalog).unwrap();
        assert!(!selected.fallback);
        assert_eq!(selected.voice, selected.preferred_voice);
    }

    /// Explicit opt-in network test using only synthetic sample text.
    #[tokio::test]
    #[ignore = "contacts Microsoft Edge speech service; writes synthetic audio under .tools"]
    async fn live_synthetic_smoke() {
        let client = SpeechClient::new().unwrap();
        let mut evidence = Vec::new();
        std::fs::create_dir_all(".tools").unwrap();
        for (language, text, file) in [
            (
                "DE",
                "Dies ist ein synthetischer Sprachtest.",
                ".tools/edge-speech-de.mp3",
            ),
            (
                "FR",
                "Ceci est un test vocal synthétique.",
                ".tools/edge-speech-fr.mp3",
            ),
        ] {
            let start = Instant::now();
            let result = client.synthesize(text, language).await.unwrap();
            let audio = STANDARD.decode(&result.audio_base64).unwrap();
            assert!(audio.len() > 1000, "no usable audio");
            assert_eq!(result.fallback, language == "FR");
            assert!(
                audio.starts_with(b"ID3")
                    || audio
                        .windows(2)
                        .any(|b| b[0] == 0xff && b[1] & 0xe0 == 0xe0),
                "not MP3"
            );
            std::fs::write(file, &audio).unwrap();
            evidence.push(serde_json::json!({
                "language": language, "voice": result.voice,
                "preferred_voice": result.preferred_voice, "locale": result.locale,
                "fallback": result.fallback, "audio_bytes": audio.len(),
                "elapsed_ms": start.elapsed().as_millis(), "mp3": file,
                "text_kind": "synthetic non-personal smoke test", "result": "passed"
            }));
        }
        std::fs::create_dir_all("docs/evidence").unwrap();
        std::fs::write("docs/evidence/edge-speech-smoke.json", serde_json::to_vec_pretty(&serde_json::json!({ "date": "2026-10-27", "transport": "msedge-tts 0.4.1 / Sec-MS-GEC v2 / rustls", "tests": evidence })).unwrap()).unwrap();
    }
}
