//! Bounded OPC package preflight. No resources are fetched or extracted to disk.
use anyhow::{Result, ensure};
use quick_xml::{NsReader, events::Event, name::ResolveResult};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek},
};

const INVALID: &str = "Office-Datei enthält ungültige oder mehrdeutige Paketpfade.";
const UNSAFE: &str =
    "Office-Datei enthält aktive Inhalte oder unsichere externe Verweise und wird nicht übersetzt.";
const XML_LIMIT: usize = 1024 * 1024;
const TOTAL_XML_LIMIT: usize = 4 * XML_LIMIT;
const TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn component(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == b'%' {
            let pair = bytes
                .get(at + 1..at + 3)
                .ok_or_else(|| anyhow::anyhow!(INVALID))?;
            let hex = std::str::from_utf8(pair)?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| anyhow::anyhow!(INVALID))?;
            // Do not allow a second decoding pass to introduce a path separator.
            ensure!(byte != b'%', INVALID);
            decoded.push(byte);
            at += 3;
        } else {
            decoded.push(byte);
            at += 1;
        }
    }
    let value = String::from_utf8(decoded)?;
    ensure!(
        !value.is_empty() && value != "." && value != ".." && !value.ends_with(['.', ' ']),
        INVALID
    );
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '?' | '#')),
        INVALID
    );
    Ok(value.to_lowercase())
}
fn name(value: &str) -> Result<String> {
    ensure!(
        value.len() <= 1024 && !value.starts_with('/') && !value.contains('\\'),
        INVALID
    );
    value
        .trim_end_matches('/')
        .split('/')
        .map(component)
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
fn safe_part(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('/').collect();
    ensure!(
        !parts.iter().any(|part| matches!(
            *part,
            "embeddings"
                | "activex"
                | "customui"
                | "webextensions"
                | "taskpanes"
                | "vbaproject.bin"
                | "vbadata.xml"
                | "customizations.xml"
        )),
        UNSAFE
    );
    Ok(())
}
fn safe_type(value: &str) -> Result<()> {
    let value = value.to_ascii_lowercase();
    ensure!(
        ![
            "macroenabled",
            "vbaproject",
            "vbadata",
            "activex",
            "oleobject"
        ]
        .iter()
        .any(|part| value.contains(part)),
        UNSAFE
    );
    Ok(())
}

// Security metadata is deliberately a shallow XML format. Namespaces are
// resolved, attribute values unescaped, DTDs and unexpected structures rejected.
fn xml(
    data: &[u8],
    namespace: &str,
    root: &str,
    mut visit: impl FnMut(&str, &HashMap<String, String>) -> Result<()>,
) -> Result<()> {
    let mut reader = NsReader::from_reader(data);
    let mut depth = 0usize;
    let mut roots = 0;
    let mut nodes = 0;
    loop {
        nodes += 1;
        ensure!(nodes <= 40_000, UNSAFE);
        let (ns, event) = reader.read_resolved_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                ensure!(
                    matches!(ns, ResolveResult::Bound(ns) if ns.as_ref() == namespace),
                    UNSAFE
                );
                let local = e.local_name();
                let local = local.as_ref();
                if depth == 0 {
                    roots += 1;
                    ensure!(roots == 1 && local == root, UNSAFE);
                } else {
                    ensure!(depth == 1, UNSAFE);
                    let mut attributes = HashMap::new();
                    for attribute in e.attributes() {
                        let attribute = attribute?;
                        let key = attribute.key.as_ref();
                        if key == "xmlns" || key.starts_with("xmlns:") {
                            continue;
                        }
                        ensure!(!key.contains(':'), UNSAFE);
                        attributes.insert(
                            key.to_owned(),
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                .into_owned(),
                        );
                    }
                    visit(local, &attributes)?;
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                ensure!(depth > 0, UNSAFE);
                depth -= 1;
            }
            Event::Text(e) => ensure!(e.as_ref().bytes().all(|b| b.is_ascii_whitespace()), UNSAFE),
            Event::Decl(_) | Event::Comment(_) => (),
            Event::Eof => {
                ensure!(depth == 0 && roots == 1, UNSAFE);
                return Ok(());
            }
            _ => anyhow::bail!(UNSAFE),
        }
    }
}
fn required<'a>(attributes: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    attributes
        .get(key)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!(UNSAFE))
}
#[derive(Default)]
struct XmlParts {
    overrides: HashSet<String>,
    extensions: HashSet<String>,
}
fn content_types(data: &[u8], extension: &str) -> Result<XmlParts> {
    let (main, expected) = if extension == "docx" {
        (
            "word/document.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        )
    } else {
        (
            "ppt/presentation.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
        )
    };
    let mut parts = XmlParts::default();
    let mut found = false;
    let mut declared = HashSet::new();
    xml(data, TYPES_NS, "Types", |kind, attributes| {
        let content_type = required(attributes, "ContentType")?;
        safe_type(content_type)?;
        let is_xml = content_type.ends_with("+xml")
            || ["application/xml", "text/xml"].contains(&content_type);
        match kind {
            "Override" => {
                let part = required(attributes, "PartName")?
                    .strip_prefix('/')
                    .ok_or_else(|| anyhow::anyhow!(INVALID))?;
                let canonical = name(part)?;
                if is_xml {
                    parts.overrides.insert(canonical.clone());
                }
                safe_part(&canonical)?;
                ensure!(declared.insert(format!("part:{canonical}")), INVALID);
                if canonical == main {
                    ensure!(part == main && content_type == expected, UNSAFE);
                    found = true;
                }
            }
            "Default" => {
                let extension = component(required(attributes, "Extension")?)?;
                if is_xml {
                    parts.extensions.insert(extension.clone());
                }
                ensure!(declared.insert(format!("extension:{extension}")), INVALID);
            }
            _ => anyhow::bail!(UNSAFE),
        }
        Ok(())
    })?;
    ensure!(found, "Dateiinhalt und Dateityp stimmen nicht überein.");
    Ok(parts)
}
fn relationships(data: &[u8], part: &str) -> Result<HashSet<String>> {
    let base = if part == "_rels/.rels" {
        ""
    } else {
        part.rsplit_once("/_rels/")
            .map(|(base, _)| base)
            .ok_or_else(|| anyhow::anyhow!(INVALID))?
    };
    let mut ids = HashSet::new();
    let mut xml_targets = HashSet::new();
    xml(data, RELS_NS, "Relationships", |kind, attributes| {
        ensure!(kind == "Relationship", UNSAFE);
        ensure!(ids.insert(required(attributes, "Id")?.to_owned()), INVALID);
        let relationship = required(attributes, "Type")?;
        let kind = relationship
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        ensure!(
            ![
                "attachedtemplate",
                "afchunk",
                "oleobject",
                "package",
                "control",
                "vbaproject",
                "vbadata",
                "activexcontrolbinary",
                "webextension",
                "webextensiontaskpanes",
                "customui"
            ]
            .contains(&kind.as_str()),
            UNSAFE
        );
        let target = required(attributes, "Target")?;
        let external = match attributes.get("TargetMode").map(String::as_str) {
            None | Some("Internal") => false,
            Some("External") => true,
            _ => anyhow::bail!(UNSAFE),
        };
        if external {
            ensure!(
                [
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                    "http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink"
                ]
                .contains(&relationship),
                UNSAFE
            );
            safe_hyperlink(target)?;
        } else {
            // Relative parent components are normal in OPC relationships, but may
            // never escape the package. Archive entry names themselves forbid them.
            let target = target.split('#').next().unwrap_or_default();
            if target.is_empty() {
                ensure!(kind == "hyperlink", UNSAFE);
                return Ok(());
            }
            let mut parts = if target.starts_with('/') || base.is_empty() {
                Vec::new()
            } else {
                base.split('/').map(str::to_owned).collect()
            };
            for part in target.strip_prefix('/').unwrap_or(target).split('/') {
                if part == ".." {
                    ensure!(parts.pop().is_some(), INVALID);
                } else if part != "." {
                    parts.push(component(part)?);
                }
            }
            ensure!(!parts.is_empty(), INVALID);
            let resolved = parts.join("/");
            safe_part(&resolved)?;
            if ![
                "image",
                "thumbnail",
                "font",
                "audio",
                "video",
                "media",
                "hdphoto",
                "printersettings",
                "hyperlink",
            ]
            .contains(&kind.as_str())
            {
                xml_targets.insert(resolved);
            }
        }
        Ok(())
    })?;
    Ok(xml_targets)
}

fn safe_hyperlink(target: &str) -> Result<()> {
    let url = reqwest::Url::parse(target)?;
    ensure!(
        ["https", "http", "mailto"].contains(&url.scheme())
            && url.username().is_empty()
            && url.password().is_none(),
        UNSAFE
    );
    Ok(())
}
fn field_instruction(instruction: &str) -> Result<()> {
    ensure!(
        !instruction
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n')),
        UNSAFE
    );
    let instruction = instruction.trim_start();
    let keyword = instruction.split_whitespace().next().unwrap_or_default();
    ensure!(
        ![
            "DDE",
            "DDEAUTO",
            "LINK",
            "INCLUDETEXT",
            "INCLUDEPICTURE",
            "DATABASE",
            "MACROBUTTON",
            "AUTOTEXT",
            "AUTOTEXTLIST"
        ]
        .contains(&keyword.to_ascii_uppercase().as_str()),
        UNSAFE
    );
    if keyword.eq_ignore_ascii_case("HYPERLINK") {
        let mut text = instruction[keyword.len()..].trim();
        let mut tokens = Vec::new();
        while !text.is_empty() {
            if let Some(quoted) = text.strip_prefix('"') {
                let (value, rest) = quoted
                    .split_once('"')
                    .ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
                tokens.push(value);
                text = rest.trim_start();
            } else {
                let end = text.find(char::is_whitespace).unwrap_or(text.len());
                tokens.push(&text[..end]);
                text = text[end..].trim_start();
            }
        }
        let mut at = 0;
        let mut target = None;
        let mut bookmark = false;
        while at < tokens.len() {
            let token = tokens[at];
            at += 1;
            match token.to_ascii_lowercase().as_str() {
                "\\l" | "\\o" | "\\t" => {
                    ensure!(
                        at < tokens.len()
                            && !tokens[at].is_empty()
                            && !tokens[at].starts_with('\\'),
                        UNSAFE
                    );
                    if token.eq_ignore_ascii_case("\\l") {
                        bookmark = true;
                    }
                    at += 1;
                }
                "\\m" | "\\n" | "\\h" => (),
                _ => {
                    ensure!(target.is_none() && !token.starts_with('\\'), UNSAFE);
                    target = Some(token);
                }
            }
        }
        if let Some(target) = target {
            safe_hyperlink(target)?;
        } else {
            ensure!(bookmark, UNSAFE);
        }
    }
    Ok(())
}
fn word_fields(data: &[u8], check: &mut impl FnMut() -> Result<()>) -> Result<()> {
    let mut reader = NsReader::from_reader(data);
    let mut fields: Vec<(String, bool)> = Vec::new();
    let mut depth = 0usize;
    let mut nodes = 0usize;
    loop {
        nodes += 1;
        if nodes.is_multiple_of(1024) {
            check()?;
        }
        ensure!(nodes <= super::MAX_XML_NODES, UNSAFE);
        let (namespace, event) = reader.read_resolved_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let start = matches!(event, Event::Start(_));
                if start {
                    depth += 1;
                    ensure!(depth <= super::MAX_XML_DEPTH, UNSAFE);
                }
                let word = matches!(namespace, ResolveResult::Bound(ns) if ["http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"].contains(&ns.as_ref()));
                if !word {
                    continue;
                }
                let local = element.local_name();
                match local.as_ref() {
                    "fldChar" | "fldSimple" => {
                        let mut attribute_value = None;
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            let (ns, local_attribute) =
                                reader.resolver().resolve_attribute(attribute.key);
                            if matches!(ns, ResolveResult::Bound(ns) if ["http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"].contains(&ns.as_ref()))
                                && local_attribute.as_ref()
                                    == if local.as_ref() == "fldChar" {
                                        "fldCharType"
                                    } else {
                                        "instr"
                                    }
                            {
                                attribute_value = Some(
                                    attribute
                                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                        .into_owned(),
                                );
                            }
                        }
                        let value = attribute_value.ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
                        if local.as_ref() == "fldSimple" {
                            field_instruction(&value)?;
                        } else {
                            match value.as_str() {
                                "begin" => {
                                    ensure!(fields.len() < 32, UNSAFE);
                                    fields.push((String::new(), false));
                                }
                                "separate" => {
                                    let field =
                                        fields.last_mut().ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
                                    field_instruction(&field.0)?;
                                    field.1 = true;
                                }
                                "end" => {
                                    let field =
                                        fields.pop().ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
                                    field_instruction(&field.0)?;
                                }
                                _ => anyhow::bail!(UNSAFE),
                            }
                        }
                    }
                    "instrText" if start => {
                        let raw = reader.read_text(element.name())?;
                        ensure!(raw.len() <= 64 * 1024 && !raw.contains('<'), UNSAFE);
                        let text = quick_xml::escape::unescape(&raw)?;
                        let field = fields.last_mut().ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
                        ensure!(
                            !field.1 && field.0.len().saturating_add(text.len()) <= 64 * 1024,
                            UNSAFE
                        );
                        field.0.push_str(&text);
                        depth -= 1;
                    }
                    _ => (),
                }
            }
            Event::End(_) => {
                ensure!(depth > 0, UNSAFE);
                depth -= 1;
            }
            Event::DocType(_) => anyhow::bail!(UNSAFE),
            Event::Eof => {
                ensure!(depth == 0 && fields.is_empty(), UNSAFE);
                return Ok(());
            }
            _ => (),
        }
    }
}
fn read_limited(
    reader: &mut impl Read,
    limit: usize,
    total: &mut usize,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        check()?;
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        *total = total.saturating_add(count);
        ensure!(
            data.len().saturating_add(count) <= limit && *total <= super::MAX_EXPANDED as usize,
            UNSAFE
        );
        data.extend_from_slice(&chunk[..count]);
    }
    Ok(data)
}
pub(super) fn validate<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    extension: &str,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    ensure!(
        archive.len() <= 10_000 && !archive.has_duplicate_names(),
        INVALID
    );
    let mut seen = HashSet::new();
    let mut xml_targets = HashSet::new();
    let mut total_xml = 0usize;
    let mut scanned = 0usize;
    let mut types = None;
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        check()?;
        let mut part = archive.by_index(index)?;
        ensure!(!part.is_symlink(), INVALID);
        expanded = expanded
            .checked_add(part.size())
            .ok_or_else(|| anyhow::anyhow!(UNSAFE))?;
        ensure!(expanded <= super::MAX_EXPANDED, UNSAFE);
        let original = part.name()?.into_owned();
        let canonical = name(&original)?;
        ensure!(seen.insert(canonical.clone()), INVALID);
        safe_part(&canonical)?;
        ensure!(
            !canonical.starts_with("_xmlsignatures/"),
            "Signierte Office-Dateien werden nicht verändert."
        );
        if canonical == "[content_types].xml" || canonical.ends_with(".rels") {
            ensure!(part.size() <= XML_LIMIT as u64, UNSAFE);
            let data = read_limited(&mut part, XML_LIMIT, &mut scanned, check)?;
            total_xml += data.len();
            ensure!(total_xml <= TOTAL_XML_LIMIT, UNSAFE);
            if canonical == "[content_types].xml" {
                ensure!(original == "[Content_Types].xml", INVALID);
                types = Some(data);
            } else {
                xml_targets.extend(relationships(&data, &canonical)?);
            }
        }
    }
    let types = content_types(
        &types.ok_or_else(|| anyhow::anyhow!("Dateiinhalt und Dateityp stimmen nicht überein."))?,
        extension,
    )?;
    // OPC parts are located by relationship and MIME type, not by their filename.
    if extension == "docx" {
        for index in 0..archive.len() {
            check()?;
            let mut part = archive.by_index(index)?;
            let canonical = name(&part.name()?)?;
            if canonical == "[content_types].xml" || canonical.ends_with(".rels") {
                continue;
            }
            let suffix = canonical.rsplit('.').next().unwrap_or_default();
            if canonical.ends_with(".xml")
                || types.overrides.contains(&canonical)
                || types.extensions.contains(suffix)
                || xml_targets.contains(&canonical)
            {
                ensure!(part.size() <= 16 * 1024 * 1024, UNSAFE);
                let data = read_limited(&mut part, 16 * 1024 * 1024, &mut scanned, check)?;
                word_fields(&data, check)?;
            }
        }
    }
    Ok(())
}
