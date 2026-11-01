//! DeepL/Linguee dictionary endpoint observed in the shipped translator web UI.
//! Remote HTML is parsed into bounded plain-text records, never forwarded to a WebView.
use anyhow::{Result, bail};
use encoding_rs::{Encoding, ISO_8859_15, UTF_8};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUERY_CHARS: usize = 120;
const CACHE_LIMIT: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, Deserialize)]
pub struct DictionaryRequest {
    pub query: String,
    pub source_language: String,
    pub target_language: String,
    pub ui_language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryResponse {
    pub query: String,
    pub entries: Vec<DictionaryEntry>,
    pub supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub word: String,
    pub word_type: String,
    pub translations: Vec<DictionaryTranslation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryTranslation {
    pub text: String,
    pub word_type: String,
    pub examples: Vec<DictionaryExample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryExample {
    pub source: String,
    pub target: String,
}

struct CacheEntry {
    key: String,
    at: Instant,
    value: DictionaryResponse,
}
pub struct DictionaryClient {
    http: reqwest::Client,
    cache: Mutex<VecDeque<CacheEntry>>,
    request_gate: Mutex<()>,
}
impl DictionaryClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("DeepLClient/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| anyhow::anyhow!("Wörterbuch konnte nicht initialisiert werden."))?,
            cache: Mutex::new(VecDeque::new()),
            request_gate: Mutex::new(()),
        })
    }

    pub async fn lookup(&self, request: &DictionaryRequest) -> Result<DictionaryResponse> {
        let query = validate_query(&request.query)?;
        let Some((source_code, source)) = language(&request.source_language) else {
            return Ok(unsupported(query));
        };
        let Some((target_code, target)) = language(&request.target_language) else {
            return Ok(unsupported(query));
        };
        if !supported_pair(source_code, target_code)
            || request.target_language.eq_ignore_ascii_case("ZH-HANT")
        {
            return Ok(unsupported(query));
        }
        let ui = match request.ui_language.to_ascii_lowercase().as_str() {
            "de" => "de",
            "en" => "en",
            "cs" => "cs",
            "fr" => "fr",
            "ja" => "ja",
            "it" => "it",
            _ => bail!("Ungültige Wörterbuchanfrage."),
        };
        let key = format!("{source_code}:{target_code}:{ui}:{query}");
        // Serializing dictionary lookups also coalesces identical simultaneous requests.
        let _guard = self.request_gate.lock().await;
        {
            let mut cache = self.cache.lock().await;
            cache.retain(|entry| entry.at.elapsed() < CACHE_TTL);
            if let Some(entry) = cache.iter().find(|entry| entry.key == key) {
                return Ok(entry.value.clone());
            }
        }
        let response = self
            .http
            .post(format!("https://dict.deepl.com/{source}-{target}/search"))
            .query(&[
                ("ajax", "1"),
                ("source", source),
                ("onlyDictEntries", "1"),
                ("translator", "dnsof7h3k2lgh3gda"),
                ("kind", "full"),
                ("eventkind", "click"),
                ("forleftside", "false"),
                ("il", ui),
            ])
            .header(reqwest::header::ACCEPT, "text/html")
            .header(reqwest::header::REFERER, "https://www.deepl.com/")
            .form(&[("query", query.as_str())])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Wörterbuch ist derzeit nicht erreichbar."))?;
        let html = read_response(response).await?;
        let value = parse_dictionary(&html, source_code, query)?;
        let mut cache = self.cache.lock().await;
        while cache.len() >= CACHE_LIMIT {
            cache.pop_front();
        }
        cache.push_back(CacheEntry {
            key,
            at: Instant::now(),
            value: value.clone(),
        });
        Ok(value)
    }
}

fn unsupported(query: String) -> DictionaryResponse {
    DictionaryResponse {
        query,
        entries: Vec::new(),
        supported: false,
    }
}
fn validate_query(input: &str) -> Result<String> {
    if input
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        bail!("Ungültige Wörterbuchanfrage.");
    }
    let query = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        bail!("Kein Wort für das Wörterbuch ausgewählt.");
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        bail!("Die Wörterbuchsuche unterstützt höchstens 120 Zeichen.");
    }
    Ok(query)
}
fn language(input: &str) -> Option<(&'static str, &'static str)> {
    Some(
        match input.to_ascii_uppercase().replace('_', "-").as_str() {
            "EN" | "EN-GB" | "EN-US" => ("EN", "english"),
            "DE" | "DE-DE" => ("DE", "german"),
            "FR" | "FR-FR" => ("FR", "french"),
            "ES" | "ES-ES" => ("ES", "spanish"),
            "IT" | "IT-IT" => ("IT", "italian"),
            "NL" | "NL-NL" => ("NL", "dutch"),
            "PL" | "PL-PL" => ("PL", "polish"),
            "RU" | "RU-RU" => ("RU", "russian"),
            "PT" | "PT-PT" | "PT-BR" => ("PT", "portuguese"),
            "JA" | "JA-JP" => ("JA", "japanese"),
            "ZH" | "ZH-HANS" | "ZH-HANT" | "ZH-CN" => ("ZH", "chinese"),
            _ => return None,
        },
    )
}
fn supported_pair(source: &str, target: &str) -> bool {
    match source {
        "EN" => matches!(
            target,
            "DE" | "FR" | "ES" | "PT" | "IT" | "NL" | "PL" | "RU" | "JA" | "ZH"
        ),
        "DE" => matches!(target, "EN" | "FR" | "ES" | "PT"),
        "FR" => matches!(target, "EN" | "DE" | "ES" | "PT"),
        "ES" => matches!(target, "EN" | "DE" | "PT"),
        "PT" => matches!(target, "EN" | "DE" | "ES"),
        "IT" | "NL" | "PL" | "RU" | "JA" | "ZH" => target == "EN",
        _ => false,
    }
}

async fn read_response(mut response: reqwest::Response) -> Result<String> {
    if !response.status().is_success() {
        bail!("Wörterbuchanfrage wurde abgelehnt.");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
    {
        bail!("Wörterbuchantwort überschreitet die Größenbegrenzung.");
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Wörterbuch ist derzeit nicht erreichbar."))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            bail!("Wörterbuchantwort überschreitet die Größenbegrenzung.");
        }
        bytes.extend_from_slice(&chunk);
    }
    decode_html(&bytes, &content_type)
}
fn decode_html(bytes: &[u8], content_type: &str) -> Result<String> {
    let header = content_type.to_ascii_lowercase();
    let charset = header
        .split(';')
        .find_map(|part| part.trim().strip_prefix("charset="))
        .map(|part| part.trim_matches(['\'', '"']));
    let marker = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
    let encoding = if let Some(label) = charset {
        match label {
            "latin-9" | "latin9" => ISO_8859_15,
            _ => Encoding::for_label(label.as_bytes())
                .ok_or_else(|| anyhow::anyhow!("Wörterbuchantwort ist ungültig."))?,
        }
    } else if marker.contains("data-lingueeencoding='latin-9'")
        || marker.contains("data-lingueeencoding=\"latin-9\"")
    {
        ISO_8859_15
    } else {
        UTF_8
    };
    let (decoded, invalid) = encoding.decode_without_bom_handling(bytes);
    if invalid {
        bail!("Wörterbuchantwort ist ungültig.");
    }
    Ok(decoded.into_owned())
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("fixed dictionary CSS selector")
}
fn plain_text(element: ElementRef<'_>, max_chars: usize) -> String {
    let text = element
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            if node.ancestors().filter_map(ElementRef::wrap).any(|parent| {
                matches!(
                    parent.value().name(),
                    "script" | "style" | "template" | "noscript"
                )
            }) {
                return None;
            }
            Some(&**text)
        })
        .collect::<String>();
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}
fn selected_text(element: ElementRef<'_>, selector: &Selector, limit: usize) -> String {
    element
        .select(selector)
        .next()
        .map(|e| plain_text(e, limit))
        .unwrap_or_default()
}
fn parse_dictionary(html: &str, source_code: &str, query: String) -> Result<DictionaryResponse> {
    let fragment = Html::parse_fragment(html);
    let data = selector("#data");
    if fragment.select(&data).next().is_none() {
        bail!("Wörterbuchantwort ist ungültig.");
    }
    let root_selector =
        selector(".isMainTerm:not(.spell_suggestion), .isForeignTerm:not(.spell_suggestion)");
    let lemma_selector = selector(".exact .lemma");
    let heading_selector = selector("h2.lemma_desc .tag_lemma .dictLink");
    let word_type_selector = selector("h2.lemma_desc .tag_wordtype");
    let translations_selector = selector(".lemma_content .translation");
    let translation_text_selector = selector(".translation_desc .tag_trans > .dictLink");
    let translation_type_selector = selector(".translation_desc .tag_trans > .tag_type");
    let example_selector = selector(".example_lines .example");
    let example_source_selector = selector(".tag_s");
    let example_target_selector = selector(".tag_t");
    let mut entries = Vec::new();
    let mut seen_entries = HashSet::new();
    for root in fragment.select(&root_selector).filter(|root| {
        root.attr("data-source-lang")
            .is_some_and(|code| code.eq_ignore_ascii_case(source_code))
    }) {
        for lemma in root.select(&lemma_selector) {
            if entries.len() >= 12 {
                break;
            }
            let word = selected_text(lemma, &heading_selector, 160);
            let word_type = selected_text(lemma, &word_type_selector, 80);
            if word.is_empty() || !seen_entries.insert((word.clone(), word_type.clone())) {
                continue;
            }
            let mut translations = Vec::new();
            let mut seen_translations = HashSet::new();
            for translation in lemma.select(&translations_selector) {
                if translations.len() >= 20 {
                    break;
                }
                let text = selected_text(translation, &translation_text_selector, 240);
                let word_type = selected_text(translation, &translation_type_selector, 80);
                if text.is_empty() || !seen_translations.insert((text.clone(), word_type.clone())) {
                    continue;
                }
                let mut examples = Vec::new();
                let mut seen_examples = HashSet::new();
                for example in translation.select(&example_selector) {
                    if examples.len() >= 3 {
                        break;
                    }
                    let source = selected_text(example, &example_source_selector, 600);
                    let target = selected_text(example, &example_target_selector, 600);
                    if !source.is_empty()
                        && !target.is_empty()
                        && seen_examples.insert((source.clone(), target.clone()))
                    {
                        examples.push(DictionaryExample { source, target });
                    }
                }
                translations.push(DictionaryTranslation {
                    text,
                    word_type,
                    examples,
                });
            }
            if !translations.is_empty() {
                entries.push(DictionaryEntry {
                    word,
                    word_type,
                    translations,
                });
            }
        }
    }
    Ok(DictionaryResponse {
        query,
        entries,
        supported: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = r#"<div id='data'></div><div class='isMainTerm' data-source-lang='EN'><div class='exact'>
      <div class='lemma'><h2 class='lemma_desc'><span class='tag_lemma'><a class='dictLink' onclick='bad()'>house &amp; home<script>alert(1)</script></a><span class='tag_wordtype'>noun</span></span></h2>
      <div class='lemma_content'><div class='translation'><h3 class='translation_desc'><span class='tag_trans'><a class='dictLink' href='javascript:bad()'>Häuschen &amp; Heim</a><span class='tag_type'>nt</span></span></h3>
      <div class='example_lines'><div class='example'><span class='tag_s'>This is <b>my</b> home.</span><span class='tag_t'>Das ist mein Zuhause.</span></div></div></div></div></div>
      <div class='lemma'><h2 class='lemma_desc'><span class='tag_lemma'><a class='dictLink'>house &amp; home</a><span class='tag_wordtype'>noun</span></span></h2><div class='lemma_content'><div class='translation'><h3 class='translation_desc'><span class='tag_trans'><a class='dictLink'>duplicate</a></span></h3></div></div></div>
      </div><div class='inexact'><div class='lemma'><h2 class='lemma_desc'><span class='tag_lemma'><a class='dictLink'>unrelated</a></span></h2></div></div></div>
      <div class='isForeignTerm' data-source-lang='DE'><div class='exact'><div class='lemma'><h2 class='lemma_desc'><span class='tag_lemma'><a class='dictLink'>wrong direction</a></span></h2></div></div></div>"#;
    #[test]
    fn parses_bounded_plain_dictionary_with_direction_and_examples() {
        let result = parse_dictionary(FIXTURE, "EN", "house".into()).unwrap();
        assert_eq!(result.entries.len(), 1);
        let entry = &result.entries[0];
        assert_eq!(entry.word, "house & home");
        assert_eq!(entry.word_type, "noun");
        assert_eq!(entry.translations[0].text, "Häuschen & Heim");
        assert_eq!(entry.translations[0].examples[0].source, "This is my home.");
        assert_eq!(
            entry.translations[0].examples[0].target,
            "Das ist mein Zuhause."
        );
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("alert(") && !json.contains("javascript:") && !json.contains("onclick")
        );
    }
    #[test]
    fn decodes_latin9_and_utf8_without_mojibake() {
        let text = "<div data-lingueeEncoding='latin-9'>große Küche, 5 €</div>";
        let (bytes, _, invalid) = ISO_8859_15.encode(text);
        assert!(!invalid);
        assert_eq!(decode_html(&bytes, "text/html").unwrap(), text);
        assert_eq!(
            decode_html(&bytes, "text/html; charset=iso-8859-15").unwrap(),
            text
        );
        assert_eq!(
            decode_html("日本語の家".as_bytes(), "text/html; charset=utf-8").unwrap(),
            "日本語の家"
        );
        assert!(decode_html(&[0xff], "text/html; charset=utf-8").is_err());
        assert!(decode_html(b"text", "text/html; charset=unknown-private").is_err());
    }
    #[test]
    fn query_validation_and_observed_pair_support() {
        assert_eq!(validate_query("  schönes\n Haus ").unwrap(), "schönes Haus");
        assert!(validate_query("\t").is_err());
        assert!(validate_query("a\0b").is_err());
        assert!(validate_query(&"界".repeat(121)).is_err());
        assert!(validate_query(&"界".repeat(120)).is_ok());
        assert_eq!(language("en-GB"), Some(("EN", "english")));
        assert_eq!(language("PT_BR"), Some(("PT", "portuguese")));
        assert!(language("CS").is_none());
        assert!(language("EN/../../other").is_none());
        assert!(supported_pair("JA", "EN"));
        assert!(!supported_pair("JA", "DE"));
        assert!(!supported_pair("DE", "DE"));
    }
    #[tokio::test]
    async fn unsupported_languages_do_not_make_network_requests() {
        let client = DictionaryClient::new().unwrap();
        for (source, target) in [("CS", "DE"), ("DE", "JA"), ("EN", "ZH-HANT")] {
            let result = client
                .lookup(&DictionaryRequest {
                    query: "house".into(),
                    source_language: source.into(),
                    target_language: target.into(),
                    ui_language: "de".into(),
                })
                .await
                .unwrap();
            assert!(!result.supported && result.entries.is_empty());
        }
    }
    #[test]
    fn empty_results_are_distinct_from_challenge_or_malformed_response() {
        assert!(
            parse_dictionary(
                "<div id='data'></div><div id='dictionary'></div>",
                "EN",
                "x".into()
            )
            .unwrap()
            .entries
            .is_empty()
        );
        assert!(
            parse_dictionary(
                "<html>challenge: secret server token</html>",
                "EN",
                "x".into()
            )
            .unwrap_err()
            .to_string()
            .contains("ungültig")
        );
        assert_eq!(
            parse_dictionary(FIXTURE, "RU", "x".into())
                .unwrap()
                .entries
                .len(),
            0
        );
    }
    #[test]
    fn dictionary_output_caps_do_not_depend_on_remote_markup_size() {
        let mut html = String::from(
            "<div id='data'></div><div class='isMainTerm' data-source-lang='EN'><div class='exact'>",
        );
        for lemma in 0..15 {
            html.push_str(&format!("<div class='lemma'><h2 class='lemma_desc'><span class='tag_lemma'><a class='dictLink'>word {lemma}</a></span></h2><div class='lemma_content'>"));
            for translation in 0..24 {
                html.push_str(&format!("<div class='translation'><h3 class='translation_desc'><span class='tag_trans'><a class='dictLink'>text {translation}</a></span></h3><div class='example_lines'>"));
                for example in 0..5 {
                    html.push_str(&format!("<div class='example'><span class='tag_s'>source {example}</span><span class='tag_t'>target {example}</span></div>"));
                }
                html.push_str("</div></div>");
            }
            html.push_str("</div></div>");
        }
        html.push_str("</div></div>");
        let result = parse_dictionary(&html, "EN", "word".into()).unwrap();
        assert_eq!(result.entries.len(), 12);
        assert!(result.entries.iter().all(|entry| {
            entry.translations.len() == 20
                && entry
                    .translations
                    .iter()
                    .all(|translation| translation.examples.len() == 3)
        }));
    }
    #[tokio::test]
    async fn actual_http_reader_enforces_size_and_sanitizes_errors() {
        use std::io::{Read, Write};
        async fn local_response(status: u16, body: Vec<u8>, chunked: bool) -> reqwest::Response {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                let _ = stream.read(&mut request);
                if chunked {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                    let _ = stream.write_all(b"\r\n0\r\n\r\n");
                    return;
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                let _ = stream.write_all(&body);
            });
            reqwest::Client::new()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap()
        }
        assert_eq!(
            read_response(local_response(200, b"<div id='data'></div>".to_vec(), false).await)
                .await
                .unwrap(),
            "<div id='data'></div>"
        );
        let error =
            read_response(local_response(200, vec![b'x'; MAX_RESPONSE_BYTES + 1], false).await)
                .await
                .unwrap_err();
        assert!(error.to_string().contains("Größenbegrenzung"));
        let error =
            read_response(local_response(200, vec![b'x'; MAX_RESPONSE_BYTES + 1], true).await)
                .await
                .unwrap_err();
        assert!(error.to_string().contains("Größenbegrenzung"));
        let error =
            read_response(local_response(403, b"private rejection payload".to_vec(), false).await)
                .await
                .unwrap_err();
        assert_eq!(error.to_string(), "Wörterbuchanfrage wurde abgelehnt.");
    }
    #[tokio::test]
    #[ignore = "contacts the real DeepL dictionary using a synthetic public word"]
    async fn live_dictionary_smoke() {
        let client = DictionaryClient::new().unwrap();
        let result = client
            .lookup(&DictionaryRequest {
                query: "house".into(),
                source_language: "EN".into(),
                target_language: "DE".into(),
                ui_language: "de".into(),
            })
            .await
            .unwrap();
        assert!(result.supported && !result.entries.is_empty());
        assert!(result.entries.iter().any(|entry| {
            entry
                .translations
                .iter()
                .any(|translation| translation.text == "Haus")
        }));
        assert!(
            result
                .entries
                .iter()
                .flat_map(|entry| &entry.translations)
                .flat_map(|translation| &translation.examples)
                .any(|example| example.target.contains("Küche"))
        );
        std::fs::create_dir_all("docs/evidence").unwrap();
        std::fs::write("docs/evidence/dictionary-smoke.json",serde_json::to_vec_pretty(&serde_json::json!({"date":"2026-10-28","query":"house","source":"EN","target":"DE","entries":result.entries.len(),"contains_Haus":true,"latin9_Kueche_preserved":true,"result":"passed"})).unwrap()).unwrap();
    }
}
