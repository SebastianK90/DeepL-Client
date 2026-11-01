//! Independent reconstruction of the observed ITA v2 wire protocol. No DeepL
//! JavaScript is executed or shipped; credentials from other apps are not used.
use super::wire::{self, Proto, bytes, number, read_version, string, version};
use anyhow::{Result, bail, ensure};
use async_tungstenite::{
    WebSocketStream,
    tokio::{ConnectStream, connect_async_with_config},
    tungstenite::{Message, protocol::WebSocketConfig},
};
use futures_util::StreamExt;
use rmpv::Value;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
const ROOT: &str = "https://ita-free.www.deepl.com";
const FAILED: &str = "Interaktive DeepL-Verbindung fehlgeschlagen. Bitte erneut versuchen.";
const INVALID: &str = "Ungültige Antwort des interaktiven Dienstes.";
const STALE: &str = "Die Wortauswahl ist nicht mehr aktuell. Bitte das Wort erneut auswählen.";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LookupMode {
    #[default]
    Translate,
    Write,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AlternativeKind {
    #[default]
    Word,
    Sentence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LookupRequest {
    pub source_text: String,
    pub target_text: String,
    pub source_language: String,
    pub target_language: String,
    pub offset: u64,
    #[serde(default)]
    mode: LookupMode,
    #[serde(default)]
    kind: AlternativeKind,
    #[serde(default)]
    style: crate::core::WritingStyle,
}
impl LookupRequest {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.source_text.trim().is_empty()
                && !self.target_text.trim().is_empty()
                && self.source_text.chars().count() <= 5000
                && self.target_text.encode_utf16().count() <= 20000,
            "Ungültige Wortauswahl."
        );
        language(&self.source_language)?;
        language(&self.target_language)?;
        if self.mode == LookupMode::Write {
            ensure!(
                self.source_language == self.target_language,
                "Ungültige Wortauswahl."
            );
            crate::core::TranslationRequest {
                text: self.source_text.clone(),
                source: "AUTO".into(),
                target: self.target_language.clone(),
                style: self.style,
            }
            .validate_write()
            .map_err(anyhow::Error::msg)?;
        } else {
            ensure!(self.kind == AlternativeKind::Word, "Ungültige Wortauswahl.");
        }
        wire::utf16_byte(&self.target_text, self.offset)?;
        ensure!(
            self.offset < self.target_text.encode_utf16().count() as u64,
            "Ungültige Wortauswahl."
        );
        Ok(())
    }
    fn same_document(&self, other: &Self) -> bool {
        self.source_text == other.source_text
            && self.target_text == other.target_text
            && self.source_language == other.source_language
            && self.target_language == other.target_language
            && self.mode == other.mode
            && self.style == other.style
    }
}
fn language(code: &str) -> Result<String> {
    let normalized = code.to_uppercase();
    ensure!(
        crate::core::LANGUAGES.contains(&normalized.as_str()) && normalized != "AUTO",
        "Ungültige Wortauswahl."
    );
    Ok(match normalized.as_str() {
        "EN" => "en-GB".into(),
        "PT" => "pt-PT".into(),
        "ZH" => "zh-Hans".into(),
        "EN-GB" => "en-GB".into(),
        "EN-US" => "en-US".into(),
        "PT-PT" => "pt-PT".into(),
        "PT-BR" => "pt-BR".into(),
        _ => normalized.to_lowercase(),
    })
}
#[derive(Clone, Serialize)]
pub struct AlternativeResponse {
    pub ticket: String,
    pub alternatives: Vec<String>,
    pub range_start: u64,
    pub range_end: u64,
}
#[derive(Clone)]
struct WordOffer {
    range: Vec<u8>,
    flow: String,
    alternatives: Vec<String>,
    request_annotation: String,
    kind: AlternativeKind,
    variants: std::collections::BTreeMap<String, Vec<u8>>,
}
struct ActiveOffer {
    ticket: String,
    request: LookupRequest,
    offer: WordOffer,
}
#[derive(Default)]
struct ClientState {
    session: Option<Session>,
    offer: Option<ActiveOffer>,
}
pub struct InteractiveClient {
    http: reqwest::Client,
    state: Mutex<ClientState>,
    epoch: std::sync::Mutex<Epoch>,
}
struct Epoch {
    value: u64,
    ticket: Option<String>,
    cancel: tokio::sync::watch::Sender<u64>,
}
impl Default for Epoch {
    fn default() -> Self {
        Self {
            value: 0,
            ticket: None,
            cancel: tokio::sync::watch::channel(0).0,
        }
    }
}
impl InteractiveClient {
    pub fn new() -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .user_agent(concat!("DeepLClient/", env!("CARGO_PKG_VERSION")))
                .build()?,
            state: Mutex::new(ClientState::default()),
            epoch: std::sync::Mutex::new(Epoch::default()),
        })
    }
    pub async fn lookup(&self, request: LookupRequest) -> Result<AlternativeResponse> {
        request.validate()?;
        let (epoch, cancel) = self.begin(None)?;
        let work = async {
            let mut state = self.state.lock().await;
            state.offer = None;
            let reusable = state.session.as_ref().is_some_and(|s| {
                s.request.same_document(&request) && s.last_used.elapsed() < Duration::from_secs(20)
            });
            let prior = if reusable {
                state.session.take()
            } else {
                state.session = None;
                None
            };
            let mut session = match prior {
                Some(s) => s,
                None => Session::open(&self.http, request.clone()).await?,
            };
            let offer = session.lookup(request.offset, request.kind).await?;
            let range = Proto::parse(&offer.range)?;
            let (range_start, range_end) = (range.num(1), range.num(2));
            ensure!(
                range_start <= range_end
                    && range_end <= request.target_text.encode_utf16().count() as u64,
                INVALID
            );
            wire::utf16_byte(&request.target_text, range_start)?;
            wire::utf16_byte(&request.target_text, range_end)?;
            let response = AlternativeResponse {
                ticket: uuid::Uuid::new_v4().to_string(),
                alternatives: offer.alternatives.clone(),
                range_start,
                range_end,
            };
            state.session = Some(session);
            state.offer = Some(ActiveOffer {
                ticket: response.ticket.clone(),
                request,
                offer,
            });
            self.publish(epoch, &response.ticket)?;
            Ok(response)
        };
        self.run(cancel, work).await
    }
    pub async fn apply(&self, ticket: &str, selection: &str) -> Result<crate::core::Translation> {
        let (_, cancel) = self.begin(Some(ticket))?;
        let work =
            async {
                let mut state = self.state.lock().await;
                ensure!(
                    state.offer.as_ref().is_some_and(|o| o.ticket == ticket
                        && o.offer.alternatives.iter().any(|v| v == selection)),
                    STALE
                );
                let active = state.offer.take().expect("offer validated");
                let prior = state.session.take();
                let mut session = match prior {
                    Some(s) if s.last_used.elapsed() < Duration::from_secs(20) => s,
                    _ => {
                        let mut s = Session::open(&self.http, active.request.clone()).await?;
                        s.lookup(active.request.offset, active.request.kind).await?;
                        s
                    }
                };
                let offer = session
                    .offer
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!(STALE))?;
                ensure!(offer.alternatives.iter().any(|v| v == selection), STALE);
                session.select(&offer, selection).await?;
                let output = crate::core::Translation {
                    text: session.target.clone(),
                    detected_source: active.request.source_language.clone(),
                };
                session.request.target_text = session.target.clone();
                state.session = Some(session);
                Ok(output)
            };
        self.run(cancel, work).await
    }
    fn begin(&self, ticket: Option<&str>) -> Result<(u64, tokio::sync::watch::Receiver<u64>)> {
        let mut epoch = self.epoch.lock().map_err(|_| anyhow::anyhow!(FAILED))?;
        if let Some(ticket) = ticket {
            ensure!(epoch.ticket.as_deref() == Some(ticket), STALE);
        }
        epoch.value = epoch.value.wrapping_add(1).max(1);
        epoch.ticket = None;
        let value = epoch.value;
        epoch.cancel.send_replace(value);
        Ok((value, epoch.cancel.subscribe()))
    }
    fn publish(&self, value: u64, ticket: &str) -> Result<()> {
        let mut epoch = self.epoch.lock().map_err(|_| anyhow::anyhow!(FAILED))?;
        ensure!(epoch.value == value, "Wortabfrage abgebrochen.");
        epoch.ticket = Some(ticket.into());
        Ok(())
    }
    async fn run<T>(
        &self,
        mut cancel: tokio::sync::watch::Receiver<u64>,
        work: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::select! {biased;_=cancel.changed()=>bail!("Wortabfrage abgebrochen."),result=tokio::time::timeout(Duration::from_secs(40),work)=>result.map_err(|_|anyhow::anyhow!(FAILED))?}
    }
}

struct Session {
    socket: WebSocketStream<ConnectStream>,
    buffer: Vec<u8>,
    participant: Vec<u8>,
    request: LookupRequest,
    state: ProtocolState,
    invocation: u64,
    last_used: Instant,
}
#[derive(Default)]
struct ProtocolState {
    mode: LookupMode,
    expected_kind: AlternativeKind,
    version: u64,
    published_version: u64,
    confirmed_version: u64,
    target: String,
    target_revision: u64,
    initialized: bool,
    idle: bool,
    idle_version: Option<u64>,
    required_version: u64,
    offer: Option<WordOffer>,
    expected_flow: String,
    selection: Option<SelectionCommit>,
}
struct SelectionCommit {
    anchor: u64,
    required_version: u64,
    edit_version: u64,
    commit_version: u64,
    edited_revision: u64,
    committed_revision: u64,
}
impl std::ops::Deref for Session {
    type Target = ProtocolState;
    fn deref(&self) -> &ProtocolState {
        &self.state
    }
}
impl std::ops::DerefMut for Session {
    fn deref_mut(&mut self) -> &mut ProtocolState {
        &mut self.state
    }
}
impl Session {
    async fn open(http: &reqwest::Client, request: LookupRequest) -> Result<Self> {
        let mut source_property = [
            number(1, 3),
            bytes(
                4,
                &bytes(1, &string(1, &language(&request.source_language)?)),
            ),
        ]
        .concat();
        let mut target_property = [
            number(1, 5),
            bytes(
                5,
                &bytes(1, &string(1, &language(&request.target_language)?)),
            ),
        ]
        .concat();
        let write = request.mode == LookupMode::Write;
        if write {
            source_property = [
                number(1, 21),
                bytes(
                    21,
                    &bytes(1, &string(1, &language(&request.target_language)?)),
                ),
            ]
            .concat();
            target_property = Vec::new();
        }
        let style_property = if write {
            request
                .style
                .value()
                .map(|style| [number(1, 24), bytes(24, &bytes(1, &string(1, style)))].concat())
        } else {
            None
        };
        let document = [
            bytes(
                1,
                &[
                    number(1, if write { 5 } else { 1 }),
                    string(2, &request.source_text),
                    bytes(4, &source_property),
                    style_property
                        .as_ref()
                        .map(|p| bytes(4, p))
                        .unwrap_or_default(),
                ]
                .concat(),
            ),
            bytes(
                1,
                &[
                    number(1, if write { 6 } else { 2 }),
                    string(2, &request.target_text),
                    if target_property.is_empty() {
                        Vec::new()
                    } else {
                        bytes(4, &target_property)
                    },
                ]
                .concat(),
            ),
        ]
        .concat();
        let body = [
            number(1, if write { 2 } else { 1 }),
            bytes(2, &document),
            bytes(3, &[]),
        ]
        .concat();
        let mut response = http
            .post(format!("{ROOT}/v2/startSession"))
            .header("Content-Type", "application/x-protobuf")
            .header("Referer", "https://www.deepl.com/")
            .body(body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!(FAILED))?;
        ensure!(
            response.status().is_success(),
            "Interaktive DeepL-Sitzung ist derzeit nicht verfügbar."
        );
        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!(FAILED))?
        {
            ensure!(data.len() + chunk.len() <= 256 * 1024, INVALID);
            data.extend_from_slice(&chunk);
        }
        let proto = Proto::parse(&data)?;
        let participant = proto.bytes(2).to_vec();
        ensure!(!participant.is_empty(), INVALID);
        let path = proto.text(5)?;
        let base = reqwest::Url::parse(ROOT)?;
        let mut url = base.join(&path).map_err(|_| anyhow::anyhow!(INVALID))?;
        ensure!(
            url.host_str() == base.host_str()
                && url.scheme() == "https"
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none(),
            INVALID
        );
        url.set_scheme("wss")
            .map_err(|_| anyhow::anyhow!(INVALID))?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::MAX_PACKET))
            .max_frame_size(Some(wire::MAX_PACKET));
        let (mut socket, _) = connect_async_with_config(url.as_str(), Some(config))
            .await
            .map_err(|_| anyhow::anyhow!(FAILED))?;
        socket
            .send(Message::Text(
                "{\"protocol\":\"messagepack\",\"version\":1}\u{1e}".into(),
            ))
            .await
            .map_err(|_| anyhow::anyhow!(FAILED))?;
        let mut buffer = Vec::new();
        loop {
            let message = socket
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!(FAILED))?
                .map_err(|_| anyhow::anyhow!(FAILED))?;
            match message {
                Message::Binary(b) => buffer.extend_from_slice(&b),
                Message::Text(t) => buffer.extend_from_slice(t.as_bytes()),
                _ => bail!(FAILED),
            }
            ensure!(buffer.len() < 8192, INVALID);
            if let Some(end) = buffer.iter().position(|b| *b == 30) {
                let hello: serde_json::Value =
                    serde_json::from_slice(&buffer[..end]).map_err(|_| anyhow::anyhow!(INVALID))?;
                ensure!(hello.as_object().is_some_and(|o| o.is_empty()), FAILED);
                buffer.drain(..=end);
                break;
            }
        }
        let mut session = Self {
            socket,
            buffer,
            participant,
            request,
            state: ProtocolState {
                mode: if write {
                    LookupMode::Write
                } else {
                    LookupMode::Translate
                },
                ..Default::default()
            },
            invocation: 0,
            last_used: Instant::now(),
        };
        session
            .invoke("Participate", Value::Ext(3, version(0)))
            .await?;
        #[cfg(test)]
        eprintln!("ITA: connected, awaiting initial state");
        // Initialized commits the replayed base document. Global Idle can be
        // delayed indefinitely by unrelated automatic sentence suggestions.
        while !session.initialized {
            session.receive().await?;
            #[cfg(test)]
            eprintln!(
                "ITA: init={} idle={} version={} target_len={}",
                session.initialized,
                session.idle,
                session.version,
                session.target.len()
            );
        }
        ensure!(
            session.target == session.request.target_text,
            "Das Ergebnis hat sich geändert. Bitte erneut übersetzen."
        );
        Ok(session)
    }
    async fn invoke(&mut self, target: &str, argument: Value) -> Result<()> {
        self.invocation += 1;
        let message = Value::Array(vec![
            1.into(),
            Value::Map(vec![]),
            self.invocation.to_string().into(),
            target.into(),
            Value::Array(vec![argument]),
            Value::Array(vec![]),
        ]);
        self.socket
            .send(Message::Binary(wire::frame(&message)?.into()))
            .await
            .map_err(|_| anyhow::anyhow!(FAILED))?;
        Ok(())
    }
    async fn append(&mut self, events: Vec<Vec<u8>>) -> Result<()> {
        self.state.prepare_append(events.len())?;
        let mut append = Vec::new();
        for event in events {
            append.extend(bytes(1, &event));
        }
        append.extend(bytes(2, &version(self.version)));
        self.idle = false;
        self.invoke("AppendRequest", Value::Ext(4, bytes(1, &append)))
            .await
    }
    fn annotation(&self, id: &str, range: &[u8], payload_tag: u32, payload: &[u8]) -> Vec<u8> {
        [
            number(
                1,
                if self.request.mode == LookupMode::Write {
                    6
                } else {
                    2
                },
            ),
            bytes(
                3,
                &[
                    bytes(1, &string(1, id)),
                    bytes(2, range),
                    bytes(payload_tag, payload),
                ]
                .concat(),
            ),
            bytes(6, &self.participant),
        ]
        .concat()
    }
    async fn lookup(&mut self, offset: u64, kind: AlternativeKind) -> Result<WordOffer> {
        #[cfg(test)]
        eprintln!("ITA: requesting alternatives");
        self.expected_flow = uuid::Uuid::new_v4().to_string();
        self.expected_kind = kind;
        let id = uuid::Uuid::new_v4().to_string();
        let range = [number(1, offset), number(2, offset)].concat();
        let payload = bytes(1, &string(1, &self.expected_flow));
        let mut events = Vec::new();
        if let Some(old) = self.offer.take() {
            events.push(
                [
                    number(
                        1,
                        if self.request.mode == LookupMode::Write {
                            6
                        } else {
                            2
                        },
                    ),
                    bytes(4, &bytes(1, &string(1, &old.request_annotation))),
                    bytes(6, &self.participant),
                ]
                .concat(),
            );
        }
        events.push(self.annotation(
            &id,
            &range,
            if kind == AlternativeKind::Sentence {
                8
            } else {
                4
            },
            &payload,
        ));
        self.append(events).await?;
        while self.offer.is_none() {
            self.receive().await?;
        }
        let mut offer = self.offer.clone().ok_or_else(|| anyhow::anyhow!(INVALID))?;
        offer.request_annotation = id;
        self.offer = Some(offer.clone());
        self.last_used = Instant::now();
        Ok(offer)
    }
    async fn select(&mut self, offer: &WordOffer, selection: &str) -> Result<()> {
        let mut payload = [bytes(1, &string(1, &offer.flow)), string(2, selection)].concat();
        if let Some(variant) = offer.variants.get(selection) {
            payload.extend(bytes(3, variant));
        }
        let event = self.annotation(
            &uuid::Uuid::new_v4().to_string(),
            &offer.range,
            if offer.kind == AlternativeKind::Sentence {
                10
            } else {
                6
            },
            &payload,
        );
        self.append(vec![event]).await?;
        self.state.begin_selection(&offer.range)?;
        // A translated text-unit annotation after a target edit commits the
        // server's complete rephrased unit. Do not return a partial text chunk.
        while !self.state.selection_complete() {
            self.receive().await?;
        }
        self.offer = None;
        self.last_used = Instant::now();
        Ok(())
    }
    async fn receive(&mut self) -> Result<()> {
        loop {
            if let Some(value) = wire::take_frame(&mut self.buffer)? {
                return self.state.process(value);
            }
            let message = tokio::time::timeout(Duration::from_secs(15), self.socket.next())
                .await
                .map_err(|_| anyhow::anyhow!(FAILED))?
                .ok_or_else(|| anyhow::anyhow!(FAILED))?
                .map_err(|_| anyhow::anyhow!(FAILED))?;
            match message {
                Message::Binary(b) => {
                    ensure!(self.buffer.len() + b.len() <= wire::MAX_PACKET * 2, INVALID);
                    self.buffer.extend_from_slice(&b);
                }
                Message::Ping(p) => self
                    .socket
                    .send(Message::Pong(p))
                    .await
                    .map_err(|_| anyhow::anyhow!(FAILED))?,
                Message::Pong(_) => {}
                _ => bail!(FAILED),
            }
        }
    }
}
impl ProtocolState {
    fn prepare_append(&mut self, count: usize) -> Result<()> {
        self.required_version = self
            .version
            .checked_add(count as u64)
            .ok_or_else(|| anyhow::anyhow!(INVALID))?;
        Ok(())
    }
    fn begin_selection(&mut self, range: &[u8]) -> Result<()> {
        let range = Proto::parse(range)?;
        self.selection = Some(SelectionCommit {
            anchor: range.num(1),
            required_version: self.required_version,
            edit_version: 0,
            commit_version: 0,
            edited_revision: 0,
            committed_revision: 0,
        });
        Ok(())
    }
    fn selection_complete(&self) -> bool {
        self.selection.as_ref().is_some_and(|s| {
            self.confirmed_version >= s.required_version
                && s.edit_version > self.confirmed_version
                && s.commit_version >= s.edit_version
                && s.edited_revision > 0
                && s.committed_revision == s.edited_revision
        })
    }
    fn process(&mut self, value: Value) -> Result<()> {
        let array = value.as_array().ok_or_else(|| anyhow::anyhow!(INVALID))?;
        let kind = array
            .first()
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!(INVALID))?;
        match kind {
            6 => Ok(()),
            7 => bail!(FAILED),
            3 => {
                ensure!(array.get(3).and_then(Value::as_u64) != Some(1), FAILED);
                Ok(())
            }
            1 => {
                ensure!(
                    array.get(3).and_then(Value::as_str) == Some("AppendResponse"),
                    FAILED
                );
                for argument in array
                    .get(4)
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!(INVALID))?
                {
                    let Value::Ext(5, data) = argument else {
                        bail!(INVALID)
                    };
                    let response = Proto::parse(data)?;
                    if response.has(1) {
                        let confirmed = Proto::parse(response.bytes(1))?;
                        self.confirmed_version = self
                            .confirmed_version
                            .max(read_version(confirmed.bytes(1))?);
                        self.version = self.version.max(self.confirmed_version);
                    }
                    if response.has(2) {
                        self.initialized = true;
                    }
                    if response.has(3) {
                        let published = Proto::parse(response.bytes(3))?;
                        self.published_version = read_version(published.bytes(2))?;
                        self.version = self.version.max(self.published_version);
                        for raw in published.all(1) {
                            self.event(raw)?;
                        }
                    }
                    if response.has(4) {
                        let meta = Proto::parse(response.bytes(4))?;
                        if meta.has(1) {
                            let idle = Proto::parse(meta.bytes(1))?;
                            self.idle_version = Some(read_version(idle.bytes(1))?);
                        }
                    }
                }
                self.idle = self
                    .idle_version
                    .is_some_and(|idle| idle >= self.version && idle >= self.required_version);
                Ok(())
            }
            _ => bail!(INVALID),
        }
    }
    fn event(&mut self, data: &[u8]) -> Result<()> {
        let event = Proto::parse(data)?;
        if event.num(1) != if self.mode == LookupMode::Write { 6 } else { 2 } {
            return Ok(());
        }
        if event.has(2) {
            let change = Proto::parse(event.bytes(2))?;
            let range = Proto::parse(change.bytes(1))?;
            wire::edit(
                &mut self.target,
                range.num(1),
                range.num(2),
                &change.text(2)?,
            )?;
            self.target_revision += 1;
            if let Some(selection) = &mut self.selection {
                let start = range.num(1);
                let end = range.num(2);
                let replacement = change.text(2)?.encode_utf16().count() as u64;
                if self.published_version > selection.required_version
                    && start <= selection.anchor
                    && end >= selection.anchor
                {
                    selection.edit_version = self.published_version;
                    selection.edited_revision = self.target_revision;
                    selection.anchor = start + replacement.min(selection.anchor - start);
                } else if selection.edit_version > 0
                    && self.published_version > selection.required_version
                {
                    selection.edited_revision = self.target_revision;
                    if end < selection.anchor {
                        selection.anchor = selection.anchor - (end - start) + replacement;
                    }
                }
            }
        }
        if event.has(3) {
            let annotation = Proto::parse(event.bytes(3))?;
            if annotation.has(3) {
                let payload = Proto::parse(annotation.bytes(3))?;
                let range = Proto::parse(annotation.bytes(2))?;
                if let Some(selection) = &mut self.selection
                    && (payload.num(2) == 1 || self.mode == LookupMode::Write)
                    && selection.edit_version > 0
                    && self.published_version >= selection.edit_version
                    && range.num(1) <= selection.anchor
                    && range.num(2) >= selection.anchor
                {
                    selection.commit_version = self.published_version;
                    selection.committed_revision = self.target_revision;
                }
            }
            let provided_tag = if self.expected_kind == AlternativeKind::Sentence {
                18
            } else {
                5
            };
            if annotation.has(provided_tag) {
                let provided = Proto::parse(annotation.bytes(provided_tag))?;
                let flow = Proto::parse(provided.bytes(1))?.text(1)?;
                if !self.expected_flow.is_empty() && flow == self.expected_flow {
                    let mut alternatives = Vec::new();
                    let mut variants = std::collections::BTreeMap::new();
                    for raw in provided.all(2).take(80) {
                        let item = Proto::parse(raw)?;
                        let text = item.text(1)?;
                        ensure!(text.len() <= 12000, INVALID);
                        if !text.trim().is_empty() && !alternatives.contains(&text) {
                            if self.expected_kind == AlternativeKind::Sentence && item.has(4) {
                                variants.insert(text.clone(), item.bytes(4).to_vec());
                            }
                            alternatives.push(text);
                        }
                    }
                    self.offer = Some(WordOffer {
                        range: annotation.bytes(2).to_vec(),
                        flow,
                        alternatives,
                        request_annotation: String::new(),
                        kind: self.expected_kind,
                        variants,
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "anonymous Write session with synthetic text; checks word and sentence offers and server-side apply"]
    async fn live_write_actions() {
        let client = InteractiveClient::new().unwrap();
        let mut records = Vec::new();
        for kind in ["word", "sentence"] {
            let request:LookupRequest=serde_json::from_value(serde_json::json!({"source_text":"Das ist ein schönes Haus. Der Garten ist groß.","target_text":"Das ist ein schönes Haus. Der Garten ist groß.","source_language":"DE","target_language":"DE","offset":12,"mode":"write","kind":kind,"style":"simple"})).unwrap();
            let offered = client.lookup(request).await.unwrap();
            assert!(!offered.alternatives.is_empty(), "{kind}: no offers");
            eprintln!("Write {kind} offers: {:?}", offered.alternatives);
            let selection = offered.alternatives[0].clone();
            let output = client.apply(&offered.ticket, &selection).await.unwrap();
            eprintln!("Write {kind} result: {}", output.text);
            assert!(!output.text.is_empty());
            assert!(output.text.contains("Garten"));
            assert!(client.apply(&offered.ticket, &selection).await.is_err());
            records.push(serde_json::json!({"kind":kind,"alternatives":offered.alternatives,"selection":selection,"result":output.text}));
        }
        std::fs::write(
            "docs/evidence/write-actions-live.json",
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn write_lookup_rejects_cross_language_and_unknown_actions() {
        let request: LookupRequest = serde_json::from_value(serde_json::json!({
            "source_text":"Ein schöner Tag.","target_text":"A nice day.",
            "source_language":"DE","target_language":"EN-GB","offset":2,
            "mode":"write","kind":"word","style":"simple"
        }))
        .unwrap();
        assert!(request.validate().is_err());
        assert!(
            serde_json::from_value::<LookupRequest>(serde_json::json!({
                "source_text":"Ein schöner Tag.","target_text":"Ein schöner Tag.",
                "source_language":"DE","target_language":"DE","offset":2,
                "mode":"write","kind":"invented"
            }))
            .is_err()
        );
    }
    #[test]
    fn write_commits_only_its_edited_target_unit_and_keeps_rephrase_metadata() {
        let mut state = ProtocolState {
            mode: LookupMode::Write,
            target: "A house.".into(),
            version: 20,
            published_version: 20,
            ..Default::default()
        };
        state.prepare_append(1).unwrap();
        state
            .begin_selection(&[number(1, 2), number(2, 2)].concat())
            .unwrap();
        state.process(confirmation(21)).unwrap();
        state
            .process(publish_events(22, vec![change(0, 8, "Wrong field.")]))
            .unwrap();
        assert_eq!(state.target, "A house.");
        fn write_event(mut event: Vec<u8>) -> Vec<u8> {
            assert_eq!(&event[..2], &[8, 2]);
            event[1] = 6;
            event
        }
        state
            .process(publish_events(
                23,
                vec![write_event(change(0, 8, "A nice house."))],
            ))
            .unwrap();
        assert!(!state.selection_complete());
        state
            .process(publish_events(
                24,
                vec![write_event(target_unit(0, 13, false))],
            ))
            .unwrap();
        assert!(state.selection_complete());
        assert_eq!(state.target, "A nice house.");
        state.expected_flow = "sentence-flow".into();
        state.expected_kind = AlternativeKind::Sentence;
        let variant = string(1, "simple");
        let annotation = [
            bytes(2, &[number(1, 0), number(2, 13)].concat()),
            bytes(
                18,
                &[
                    bytes(1, &string(1, "sentence-flow")),
                    bytes(
                        2,
                        &[string(1, "A lovely house."), bytes(4, &variant)].concat(),
                    ),
                ]
                .concat(),
            ),
        ]
        .concat();
        state
            .event(&[number(1, 6), bytes(3, &annotation)].concat())
            .unwrap();
        let offer = state.offer.unwrap();
        assert_eq!(offer.kind, AlternativeKind::Sentence);
        assert_eq!(offer.alternatives, vec!["A lovely house."]);
        assert_eq!(offer.variants.get("A lovely house."), Some(&variant));
    }
    #[tokio::test]
    async fn newer_lookup_cancels_old_work_and_tickets_are_single_use() {
        let client = InteractiveClient::new().unwrap();
        let (old, cancel) = client.begin(None).unwrap();
        let (new, _) = client.begin(None).unwrap();
        assert!(client.publish(old, "old").is_err());
        assert!(
            client
                .run::<()>(cancel, std::future::pending())
                .await
                .is_err()
        );
        client.publish(new, "current").unwrap();
        assert!(client.begin(Some("wrong")).is_err());
        assert!(client.begin(Some("current")).is_ok());
        assert!(client.begin(Some("current")).is_err());
    }
    fn publish_events(at: u64, events: Vec<Vec<u8>>) -> Value {
        let mut published = Vec::new();
        for event in events {
            published.extend(bytes(1, &event));
        }
        published.extend(bytes(2, &version(at)));
        Value::Array(vec![
            1.into(),
            Value::Map(vec![]),
            Value::Nil,
            "AppendResponse".into(),
            Value::Array(vec![Value::Ext(5, bytes(3, &published))]),
            Value::Array(vec![]),
        ])
    }
    fn confirmation(at: u64) -> Value {
        Value::Array(vec![
            1.into(),
            Value::Map(vec![]),
            Value::Nil,
            "AppendResponse".into(),
            Value::Array(vec![Value::Ext(5, bytes(1, &bytes(1, &version(at))))]),
            Value::Array(vec![]),
        ])
    }
    fn change(start: u64, end: u64, text: &str) -> Vec<u8> {
        [
            number(1, 2),
            bytes(
                2,
                &[
                    bytes(1, &[number(1, start), number(2, end)].concat()),
                    string(2, text),
                ]
                .concat(),
            ),
        ]
        .concat()
    }
    fn target_unit(start: u64, end: u64, user_edited: bool) -> Vec<u8> {
        let payload = [
            bytes(1, &string(1, "test-unit")),
            number(2, u64::from(user_edited)),
        ]
        .concat();
        [
            number(1, 2),
            bytes(
                3,
                &[
                    bytes(2, &[number(1, start), number(2, end)].concat()),
                    bytes(3, &payload),
                ]
                .concat(),
            ),
        ]
        .concat()
    }
    #[test]
    fn selection_commit_replays_full_unit_and_rejects_stale_or_unrelated_units() {
        let mut state = ProtocolState {
            target: "First. A house. Last.".into(),
            version: 40,
            published_version: 40,
            ..Default::default()
        };
        state.prepare_append(1).unwrap();
        state
            .begin_selection(&[number(1, 10), number(2, 10)].concat())
            .unwrap();
        state
            .process(publish_events(40, vec![target_unit(7, 15, true)]))
            .unwrap();
        assert!(!state.selection_complete());
        state.process(confirmation(41)).unwrap();
        assert!(!state.selection_complete());
        state
            .process(publish_events(42, vec![change(7, 15, "A very")]))
            .unwrap();
        assert_eq!(state.target, "First. A very Last.");
        assert!(!state.selection_complete());
        state
            .process(publish_events(43, vec![target_unit(0, 6, true)]))
            .unwrap();
        assert!(
            !state.selection_complete(),
            "another sentence must not commit this selection"
        );
        state
            .process(publish_events(44, vec![change(13, 13, " nice house.")]))
            .unwrap();
        assert_eq!(state.target, "First. A very nice house. Last.");
        assert!(!state.selection_complete());
        state
            .process(publish_events(45, vec![target_unit(7, 25, true)]))
            .unwrap();
        assert!(state.selection_complete());
        assert_eq!(state.target, "First. A very nice house. Last.");
    }
    #[test]
    fn selection_commit_requires_ack_and_newer_published_edit() {
        let mut state = ProtocolState {
            target: "A house.".into(),
            version: 20,
            published_version: 20,
            ..Default::default()
        };
        state.prepare_append(1).unwrap();
        state
            .begin_selection(&[number(1, 2), number(2, 2)].concat())
            .unwrap();
        state
            .process(publish_events(22, vec![change(0, 8, "A nice house.")]))
            .unwrap();
        state
            .process(publish_events(23, vec![target_unit(0, 13, true)]))
            .unwrap();
        assert!(
            !state.selection_complete(),
            "commit without selection acknowledgement is premature"
        );
        state.process(confirmation(20)).unwrap();
        assert!(!state.selection_complete());
        state.process(confirmation(21)).unwrap();
        assert!(state.selection_complete());
        let mut queued = ProtocolState {
            target: "A house.".into(),
            version: 20,
            published_version: 20,
            ..Default::default()
        };
        queued.prepare_append(1).unwrap();
        queued
            .begin_selection(&[number(1, 2), number(2, 2)].concat())
            .unwrap();
        queued
            .process(publish_events(22, vec![change(0, 8, "A nice house.")]))
            .unwrap();
        queued
            .process(publish_events(23, vec![target_unit(0, 13, true)]))
            .unwrap();
        queued.process(confirmation(24)).unwrap();
        assert!(
            !queued.selection_complete(),
            "edits queued before the actual append acknowledgement cannot finish selection"
        );
    }
    #[test]
    fn request_rejects_invalid_languages_and_surrogate_offsets() {
        let mut request = LookupRequest {
            source_text: "Hallo".into(),
            target_text: "Hi 🌍".into(),
            source_language: "DE".into(),
            target_language: "EN-GB".into(),
            offset: 3,
            mode: LookupMode::Translate,
            kind: AlternativeKind::Word,
            style: Default::default(),
        };
        assert!(request.validate().is_ok());
        request.offset = 4;
        assert!(request.validate().is_err());
        request.offset = 3;
        request.target_language = "AUTO".into();
        assert!(request.validate().is_err());
    }
    #[tokio::test]
    #[ignore = "real anonymous ITA session with synthetic text and full-sentence rephrasing"]
    async fn live_contextual_alternatives() {
        let client = InteractiveClient::new().unwrap();
        let request = LookupRequest {
            source_text: "Das ist ein schönes Haus.".into(),
            target_text: "This is a beautiful house.".into(),
            source_language: "DE".into(),
            target_language: "EN-GB".into(),
            offset: 12,
            mode: LookupMode::Translate,
            kind: AlternativeKind::Word,
            style: Default::default(),
        };
        let offered = client.lookup(request).await.unwrap();
        assert!(!offered.alternatives.is_empty());
        assert!(offered.alternatives.contains(&"very nice".into()));
        let result = client.apply(&offered.ticket, "very nice").await.unwrap();
        assert_eq!(result.text, "This is a very nice house.");
        assert!(client.apply(&offered.ticket, "very nice").await.is_err());
        std::fs::write("docs/evidence/contextual-alternatives-smoke.json",serde_json::to_vec_pretty(&serde_json::json!({"date":"2026-10-29","source":"Das ist ein schönes Haus.","before":"This is a beautiful house.","word":"beautiful","alternatives":offered.alternatives,"selection":"very nice","after":result.text,"protocol":"ITA v2 / SignalR MessagePack with Protobuf extensions","result":"passed"})).unwrap()).unwrap();
    }
}
