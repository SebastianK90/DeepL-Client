//! Local document container processing. Only extracted text reaches a provider.
use anyhow::{Result, bail, ensure};
use std::io::{Cursor, Read, Write};

mod office_security;
pub mod pdf;
pub(crate) mod pdf_worker;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_XML_NODES: usize = 250_000;
const MAX_XML_DEPTH: usize = 256;
const MAX_STYLE_BYTES: usize = 64 * 1024;
const MAX_TRANSLATION_BYTES: usize = 20_000;

// Cumulative allocation/work allowance; charge before allocating, including ZIP output.
#[derive(Clone)]
struct Budget(std::rc::Rc<std::cell::Cell<usize>>);
impl Budget {
    fn new() -> Self {
        Self(std::rc::Rc::new(std::cell::Cell::new(0)))
    }
    fn charge(&self, bytes: usize) -> Result<()> {
        let next = self
            .0
            .get()
            .checked_add(bytes)
            .ok_or_else(|| anyhow::anyhow!("Dokument zu groß."))?;
        ensure!(
            next <= MAX_EXPANDED as usize,
            "Dokumentverarbeitung überschreitet 128 MB."
        );
        self.0.set(next);
        Ok(())
    }
}
struct LimitedOutput {
    inner: Cursor<Vec<u8>>,
    budget: Budget,
}
impl Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .inner
            .position()
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("Ausgabe zu groß."))?;
        let growth = end.saturating_sub(self.inner.get_ref().len() as u64);
        self.budget
            .charge(usize::try_from(growth).map_err(std::io::Error::other)?)
            .map_err(std::io::Error::other)?;
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl std::io::Seek for LimitedOutput {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.inner, pos)
    }
}

#[cfg(test)]
pub fn translate(
    bytes: Vec<u8>,
    extension: &str,
    translator: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<u8>> {
    translate_cancellable(bytes, extension, translator, &mut || Ok(()))
}

pub fn translate_cancellable(
    bytes: Vec<u8>,
    extension: &str,
    mut translator: impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    check()?;
    ensure!(bytes.len() <= MAX_BYTES, "Maximal 32 MB pro Dokument.");
    match extension {
        "pdf" => pdf_worker::translate(bytes, translator, check),
        "docx" | "pptx" => office(bytes, extension, &mut translator, check),
        _ => bail!("Bitte PDF, DOCX oder PPTX auswählen."),
    }
}

fn office(
    bytes: Vec<u8>,
    ext: &str,
    translator: &mut impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    let budget = Budget::new();
    budget.charge(bytes.len())?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(
        archive.len() <= 10_000,
        "Zu viele Bestandteile im Dokument."
    );
    let required = if ext == "docx" {
        "word/document.xml"
    } else {
        "ppt/presentation.xml"
    };
    ensure!(
        archive.index_for_name(required).is_some(),
        "Dateiinhalt und Dateityp stimmen nicht überein."
    );
    office_security::validate(&mut archive, ext, check)?;
    let mut output = zip::ZipWriter::new(LimitedOutput {
        inner: Cursor::new(Vec::new()),
        budget: budget.clone(),
    });
    let mut expanded = 0u64;
    let mut count = 0;
    let mut nodes = 0;
    for i in 0..archive.len() {
        check()?;
        let mut part = archive.by_index(i)?;
        ensure!(
            !part.name()?.starts_with("_xmlsignatures/"),
            "Signierte Office-Dateien werden nicht verändert."
        );
        expanded = expanded
            .checked_add(part.size())
            .ok_or_else(|| anyhow::anyhow!("Dokument zu groß."))?;
        ensure!(
            expanded <= MAX_EXPANDED,
            "Entpacktes Dokument überschreitet 128 MB."
        );
        let name = part.name()?.into_owned();
        let editable = if ext == "docx" {
            name.starts_with("word/")
                && ["document", "header", "footer", "footnotes", "endnotes"]
                    .iter()
                    .any(|p| name.trim_start_matches("word/").starts_with(p))
        } else {
            name.starts_with("ppt/slides/slide")
                || name.starts_with("ppt/notesSlides/notesSlide")
                || name.starts_with("ppt/charts/chart")
        } && name.ends_with(".xml");
        if editable {
            ensure!(
                part.size() <= 16 * 1024 * 1024,
                "XML-Bestandteil ist zu groß."
            );
            let mut data = Vec::new();
            let mut chunk = [0u8; 16 * 1024];
            loop {
                check()?;
                let n = part.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                ensure!(
                    data.len() + n <= 16 * 1024 * 1024,
                    "XML-Bestandteil ist zu groß."
                );
                budget.charge(n)?;
                data.extend_from_slice(&chunk[..n]);
            }
            let (data, n) = xml_limited(&data, translator, check, &budget, &mut nodes)?;
            count += n;
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(part.compression())
                .last_modified_time(part.last_modified().unwrap_or_default());
            output.start_file(name, options)?;
            output.write_all(&data)?;
        } else {
            // Compressed images, layouts, relations and binary payloads are copied verbatim.
            output.raw_copy_file(part)?;
        }
    }
    ensure!(count > 0, "Kein übersetzbarer Text im Dokument gefunden.");
    check()?;
    Ok(output.finish()?.inner.into_inner())
}

// Change only text nodes. Never redistribute text across formatting, links,
// fields, explicit line breaks, tabs, paragraphs or nested textboxes.
#[derive(Clone)]
struct TextSpan {
    range: std::ops::Range<usize>,
    content_start: usize,
}
fn boundary(name: &str) -> bool {
    matches!(
        name,
        "w:p"
            | "a:p"
            | "w:hyperlink"
            | "w:sdt"
            | "w:sdtContent"
            | "w:tab"
            | "w:br"
            | "w:cr"
            | "a:br"
            | "w:drawing"
            | "w:pict"
            | "w:object"
            | "w:txbxContent"
            | "w:sym"
            | "w:noBreakHyphen"
            | "w:softHyphen"
            | "w:bookmarkStart"
            | "w:bookmarkEnd"
            | "w:commentRangeStart"
            | "w:commentRangeEnd"
            | "w:commentReference"
            | "w:footnoteReference"
            | "w:endnoteReference"
    )
}
fn field_change(e: &quick_xml::events::BytesStart<'_>) -> Result<i32> {
    for attr in e.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == "w:fldCharType" {
            return Ok(match attr.value.as_ref() {
                "begin" => 1,
                "end" => -1,
                _ => 0,
            });
        }
    }
    Ok(0)
}
#[cfg(test)]
fn xml(
    data: &[u8],
    translator: &mut impl FnMut(&str) -> Result<String>,
) -> Result<(Vec<u8>, usize)> {
    xml_limited(data, translator, &mut || Ok(()), &Budget::new(), &mut 0)
}

fn xml_limited(
    data: &[u8],
    translator: &mut impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
    budget: &Budget,
    nodes: &mut usize,
) -> Result<(Vec<u8>, usize)> {
    use quick_xml::{Reader, events::Event};
    // Visit every event, including skipped fields/styles and whitespace. The normal
    // parser can then skip subtrees without hiding hostile depth or node counts.
    let mut scan = Reader::from_reader(data);
    let mut depth = 0usize;
    loop {
        check()?;
        let event = scan.read_event()?;
        *nodes += 1;
        budget.charge(128)?;
        ensure!(*nodes <= MAX_XML_NODES, "Zu viele XML-Knoten im Dokument.");
        match event {
            Event::Start(_) => {
                depth += 1;
                ensure!(depth <= MAX_XML_DEPTH, "XML zu tief verschachtelt.");
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
            }
            Event::DocType(_) => bail!("XML-Dokumenttypen werden nicht unterstützt."),
            Event::Eof => break,
            _ => {}
        }
    }
    let input = std::str::from_utf8(data)?;
    let mut reader = Reader::from_str(input);
    let mut spans = Vec::new();
    let mut edits = Vec::new();
    let mut run_styles: Vec<usize> = Vec::new();
    let mut styles = std::collections::HashMap::<&str, usize>::new();
    let mut pending_style = 0;
    let mut field_depth = 0i32;
    loop {
        check()?;
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) => {
                let name = e.name().as_ref().to_owned();
                if boundary(&name) {
                    flush(input, &mut spans, &mut edits, translator, check, budget)?;
                }
                match name.as_str() {
                    "w:r" | "a:r" => run_styles.push(0),
                    "w:rPr" | "a:rPr" => {
                        reader.read_to_end(e.name())?;
                        if let Some(style) = run_styles.last_mut() {
                            let value = &input[start..reader.buffer_position() as usize];
                            ensure!(
                                value.len() <= MAX_STYLE_BYTES,
                                "Run-Formatierung ist zu groß."
                            );
                            let next = styles.len() + 1;
                            *style = *styles.entry(value).or_insert(next);
                        }
                    }
                    "w:fldChar" => {
                        flush(input, &mut spans, &mut edits, translator, check, budget)?;
                        field_depth = (field_depth + field_change(&e)?).max(0);
                        reader.read_to_end(e.name())?;
                    }
                    "w:fldSimple" | "w:instrText" | "w:del" | "a:fld" => {
                        flush(input, &mut spans, &mut edits, translator, check, budget)?;
                        reader.read_to_end(e.name())?;
                    }
                    "w:t" | "a:t" => {
                        let content_start = reader.buffer_position() as usize;
                        let content = reader.read_to_end(e.name())?;
                        let end = content.end as usize;
                        ensure!(end >= content_start, "Ungültiger Textknoten.");
                        if field_depth == 0 {
                            let style = run_styles.last().copied().unwrap_or_default();
                            if !spans.is_empty() && style != pending_style {
                                flush(input, &mut spans, &mut edits, translator, check, budget)?;
                            }
                            pending_style = style;
                            spans.push(TextSpan {
                                range: start..end,
                                content_start,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_owned();
                if boundary(&name) {
                    flush(input, &mut spans, &mut edits, translator, check, budget)?;
                }
                match name.as_str() {
                    "w:rPr" | "a:rPr" => {
                        if let Some(style) = run_styles.last_mut() {
                            let value = &input[start..reader.buffer_position() as usize];
                            ensure!(
                                value.len() <= MAX_STYLE_BYTES,
                                "Run-Formatierung ist zu groß."
                            );
                            let next = styles.len() + 1;
                            *style = *styles.entry(value).or_insert(next);
                        }
                    }
                    "w:fldChar" => {
                        flush(input, &mut spans, &mut edits, translator, check, budget)?;
                        field_depth = (field_depth + field_change(&e)?).max(0);
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name().as_ref().to_owned();
                if boundary(&name) {
                    flush(input, &mut spans, &mut edits, translator, check, budget)?;
                }
                if matches!(name.as_str(), "w:r" | "a:r") {
                    run_styles.pop();
                }
            }
            Event::DocType(_) => bail!("XML-Dokumenttypen werden nicht unterstützt."),
            Event::Eof => break,
            _ => {}
        }
    }
    flush(input, &mut spans, &mut edits, translator, check, budget)?;
    let removed: usize = edits.iter().map(|(range, _)| range.len()).sum();
    let added: usize = edits.iter().map(|(_, text)| text.len()).sum();
    let length = data
        .len()
        .checked_sub(removed)
        .and_then(|v| v.checked_add(added))
        .ok_or_else(|| anyhow::anyhow!("Ausgabe zu groß."))?;
    budget.charge(length)?;
    let mut output = Vec::with_capacity(length);
    let mut cursor = 0;
    for (range, text) in &edits {
        check()?;
        output.extend_from_slice(&data[cursor..range.start]);
        output.extend_from_slice(text.as_bytes());
        cursor = range.end;
    }
    output.extend_from_slice(&data[cursor..]);
    Ok((output, edits.len()))
}

fn flush(
    input: &str,
    spans: &mut Vec<TextSpan>,
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
    translator: &mut impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
    budget: &Budget,
) -> Result<()> {
    check()?;
    if spans.is_empty() {
        return Ok(());
    }
    let mut original = String::new();
    for span in spans.iter() {
        check()?;
        budget.charge(span.range.end - span.content_start)?;
        original.push_str(&quick_xml::escape::unescape(
            &input[span.content_start..span.range.end],
        )?);
    }
    let trimmed = original.trim();
    if trimmed.chars().any(char::is_alphabetic) {
        ensure!(
            trimmed.chars().count() <= 5000,
            "Ein Textbereich überschreitet 5.000 Zeichen."
        );
        check()?;
        let translated = translator(trimmed)?;
        check()?;
        ensure!(
            translated.len() <= MAX_TRANSLATION_BYTES,
            "Übersetzung ist zu groß."
        );
        budget.charge(translated.len().saturating_mul(3))?;
        ensure!(!translated.trim().is_empty(), "Leere Übersetzung erhalten.");
        // Service-added newlines must never replace structural Word breaks/tabs.
        let translated = translated.trim().replace(['\r', '\n', '\t'], " ");
        let leading = &original[..original.len() - original.trim_start().len()];
        let trailing = &original[original.trim_end().len()..];
        budget.charge(leading.len() + translated.len() + trailing.len())?;
        let replacement = format!("{leading}{translated}{trailing}");
        // Runs in this group have identical direct formatting and no intervening
        // layout/annotation boundary. Keep the complete word stream in its first
        // text node instead of cutting words or guessing where formatting belongs.
        for (index, span) in spans.iter().enumerate() {
            check()?;
            let text = if index == 0 { replacement.as_str() } else { "" };
            budget.charge(span.content_start - span.range.start + 32)?;
            let tag = preserve_space_tag(&input[span.range.start..span.content_start])?;
            budget.charge(tag.len() + text.len().saturating_mul(12))?;
            edits.push((
                span.range.clone(),
                format!("{tag}{}", quick_xml::escape::escape(text)),
            ));
        }
    }
    spans.clear();
    Ok(())
}

fn preserve_space_tag(source: &str) -> Result<String> {
    use quick_xml::{
        Reader,
        events::{BytesStart, Event},
    };
    let mut reader = Reader::from_str(source);
    let Event::Start(original) = reader.read_event()? else {
        bail!("Ungültiger Textknoten.")
    };
    let mut tag = BytesStart::new(original.name().as_ref().to_owned());
    for attr in original.attributes() {
        let attr = attr?;
        if attr.key.as_ref() != "xml:space" {
            tag.push_attribute(attr);
        }
    }
    tag.push_attribute(("xml:space", "preserve"));
    Ok(format!("<{}>", tag.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancels_whitespace_without_translation_callback() {
        let source = format!("<w:p><w:r>{}</w:r></w:p>", "<w:t> </w:t>".repeat(500));
        // Abort both during the safety scan and after it, during text traversal.
        for stop in [100, 1600] {
            let mut visits = 0;
            let result = xml_limited(
                source.as_bytes(),
                &mut |_| unreachable!(),
                &mut || {
                    visits += 1;
                    ensure!(visits < stop, "cancelled");
                    Ok(())
                },
                &Budget::new(),
                &mut 0,
            );
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            assert_eq!(visits, stop);
        }
    }
    #[test]
    fn rejects_excessive_xml_nodes_in_skipped_style() {
        let source = format!("<w:rPr>{}</w:rPr>", "<x/>".repeat(MAX_XML_NODES));
        let result = xml(source.as_bytes(), &mut |_| unreachable!());
        assert!(result.unwrap_err().to_string().contains("XML-Knoten"));
    }
    #[test]
    fn aggregate_budget_rejects_many_small_translations_incrementally() {
        let budget = Budget::new();
        budget.charge(MAX_EXPANDED as usize - 24_000).unwrap();
        let source = "<w:p><w:r><w:t>Hello</w:t></w:r></w:p>".repeat(20);
        let mut calls = 0;
        let result = xml_limited(
            source.as_bytes(),
            &mut |_| {
                calls += 1;
                Ok("&".repeat(50))
            },
            &mut || Ok(()),
            &budget,
            &mut 0,
        );
        assert!(result.unwrap_err().to_string().contains("128 MB"));
        assert!(calls > 0 && calls < 20);
        assert!(budget.0.get() <= MAX_EXPANDED as usize);
    }
    #[test]
    fn output_writer_rejects_growth_before_allocation() {
        let budget = Budget::new();
        budget.charge(MAX_EXPANDED as usize - 3).unwrap();
        let mut writer = LimitedOutput {
            inner: Cursor::new(Vec::new()),
            budget,
        };
        assert!(writer.write_all(b"four").is_err());
        assert!(writer.inner.get_ref().is_empty());
    }
    #[test]
    fn rejects_adversarial_style_and_depth_before_translation() {
        let source = format!(
            "<w:r><w:rPr>{}</w:rPr><w:t> </w:t></w:r>",
            "x".repeat(65_537)
        );
        assert!(xml(source.as_bytes(), &mut |_| unreachable!()).is_err());
        let source = format!("{}{}", "<x>".repeat(257), "</x>".repeat(257));
        assert!(xml(source.as_bytes(), &mut |_| unreachable!()).is_err());
    }
    #[test]
    fn rejects_oversized_translation_before_xml_escaping() {
        assert!(
            xml(b"<w:p><w:r><w:t>Hello</w:t></w:r></w:p>", &mut |_| Ok(
                "&".repeat(100_000)
            ))
            .is_err()
        );
    }
    #[test]
    fn nested_textboxes_and_table_cells_stay_separate() -> Result<()> {
        let source=br#"<w:body><w:p><w:r><w:t>Before</w:t><w:drawing><w:txbxContent><w:p><w:r><w:t>Inside</w:t></w:r></w:p></w:txbxContent></w:drawing><w:t>After</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body>"#;
        let mut calls = Vec::new();
        let (out, _) = xml(source, &mut |s| {
            calls.push(s.to_owned());
            Ok(format!("[{s}]"))
        })?;
        assert_eq!(calls, ["Before", "Inside", "After", "Cell"]);
        let out = String::from_utf8(out)?;
        assert!(out.contains("<w:tblW w:w=\"4000\" w:type=\"dxa\"/>"));
        assert!(out.contains(">[Inside]</w:t>"));
        Ok(())
    }
    #[test]
    fn text_attributes_and_edge_whitespace_remain_valid() -> Result<()> {
        let source =
            br#"<w:p><w:r><w:t xml:space = 'default'>  Hello &amp; world  </w:t ></w:r></w:p>"#;
        let (out, _) = xml(source, &mut |s| {
            assert_eq!(s, "Hello & world");
            Ok("Hallo & Welt".into())
        })?;
        let out = String::from_utf8(out)?;
        assert_eq!(out.matches("xml:space").count(), 1);
        assert!(out.contains(">  Hallo &amp; Welt  </w:t >"));
        Ok(())
    }
    #[test]
    fn formatting_and_layout_boundaries_do_not_share_translation() -> Result<()> {
        let source=br#"<w:p><w:r><w:t>Click </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>Save</w:t></w:r><w:r><w:t> now</w:t><w:tab/><w:t>Column</w:t><w:br/><w:t>Next line</w:t></w:r><w:hyperlink r:id="rId7"><w:r><w:t>Link</w:t></w:r></w:hyperlink></w:p>"#;
        let mut calls = Vec::new();
        let (output, _) = xml(source, &mut |s| {
            calls.push(s.to_owned());
            Ok(match s {
                "Click" => "Klicken",
                "Save" => "Speichern",
                "now" => "jetzt",
                "Column" => "Spalte",
                "Next line" => "Nächste Zeile",
                "Link" => "Verweis",
                _ => panic!("Unexpected merged text: {s}"),
            }
            .into())
        })?;
        assert_eq!(
            calls,
            ["Click", "Save", "now", "Column", "Next line", "Link"]
        );
        let output = String::from_utf8(output)?;
        assert!(
            output.contains("<w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">Speichern</w:t>")
        );
        assert!(
            output.contains("jetzt</w:t><w:tab/><w:t xml:space=\"preserve\">Spalte</w:t><w:br/>")
        );
        assert!(output.contains("r:id=\"rId7\""));
        assert!(output.contains(">Klicken </w:t>"));
        assert!(output.contains("> jetzt</w:t>"));
        Ok(())
    }
    #[test]
    fn powerpoint_dynamic_fields_stay_unchanged_and_separate_text_runs() -> Result<()> {
        let source = br#"<a:p><a:r><a:t>Before</a:t></a:r><a:fld id="date" type="datetime1"><a:t>Monday</a:t></a:fld><a:r><a:t>After</a:t></a:r></a:p>"#;
        let mut calls = Vec::new();
        let (output, _) = xml(source, &mut |text| {
            calls.push(text.to_string());
            Ok(format!("[{text}]"))
        })?;
        assert_eq!(calls, ["Before", "After"]);
        let output = String::from_utf8(output)?;
        assert!(output.contains(r#"<a:fld id="date" type="datetime1"><a:t>Monday</a:t></a:fld>"#));
        Ok(())
    }

    #[test]
    fn identical_run_formatting_keeps_words_together_and_fields_untouched() -> Result<()> {
        let source=br#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>trans</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>lation</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Page one</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
        let mut calls = Vec::new();
        let (out, _) = xml(source, &mut |s| {
            calls.push(s.to_owned());
            Ok("Übersetzung".into())
        })?;
        assert_eq!(calls, ["translation"]);
        let out = String::from_utf8(out)?;
        assert!(out.contains("<w:t>Page one</w:t>"));
        assert!(out.contains("<w:instrText> PAGE </w:instrText>"));
        assert!(out.contains("Übersetzung"));
        Ok(())
    }
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "Live DeepL Web translation of synthetic PDF, DOCX and PPTX text; requires PDFium"]
    async fn live_local_document_formats() -> Result<()> {
        use crate::{
            core::{TranslationRequest, WritingStyle},
            provider::{TranslationProvider, web::WebProvider},
        };
        use pdfium_render::prelude::*;
        let mut inputs = Vec::new();
        {
            let pdfium = pdf::library()?;
            let mut doc = pdfium.create_new_pdf()?;
            let font = doc.fonts_mut().helvetica();
            {
                let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
                page.objects_mut().create_text_object(
                    PdfPoints::new(50.),
                    PdfPoints::new(700.),
                    "Hello world",
                    font,
                    PdfPoints::new(18.),
                )?;
            }
            inputs.push(("pdf", doc.save_to_bytes()?));
        }
        for ext in ["docx", "pptx"] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let (main, content) = if ext == "docx" {
                ("word/document.xml", "word/header1.xml")
            } else {
                ("ppt/presentation.xml", "ppt/slides/slide1.xml")
            };
            for (name, data) in [
                (main, "<root/>"),
                (
                    content,
                    "<a:p xmlns:a=\"a\"><a:r><a:t>Hello world</a:t></a:r></a:p>",
                ),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())?;
                zip.write_all(data.as_bytes())?;
            }
            let kind = if ext == "docx" {
                "wordprocessingml.document"
            } else {
                "presentationml.presentation"
            };
            zip.start_file(
                "[Content_Types].xml",
                zip::write::SimpleFileOptions::default(),
            )?;
            write!(
                zip,
                "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/{main}\" ContentType=\"application/vnd.openxmlformats-officedocument.{kind}.main+xml\"/></Types>"
            )?;
            inputs.push((ext, zip.finish()?.into_inner()));
        }
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let provider = WebProvider::new(uuid::Uuid::new_v4())?;
            let mut evidence = Vec::new();
            for (ext, input) in inputs {
                let mut texts = Vec::new();
                let output = translate(input, ext, |text| {
                    let request = TranslationRequest {
                        text: text.into(),
                        source: "EN".into(),
                        target: "DE".into(),
                        style: WritingStyle::Default,
                    };
                    let result = runtime.block_on(provider.translate(&request))?;
                    ensure!(
                        result.text != "Hello world" && !result.text.trim().is_empty(),
                        "No translation received"
                    );
                    texts.push(result.text.clone());
                    Ok(result.text)
                })?;
                std::fs::write(format!(".tools/live-local-document.{ext}"), &output)?;
                evidence.push(
                    serde_json::json!({"format":ext,"bytes":output.len(),"translations":texts}),
                );
            }
            std::fs::write(
                "docs/evidence/local-documents-live.json",
                serde_json::to_vec_pretty(&evidence)?,
            )?;
            Ok(())
        })
        .await??;
        Ok(())
    }
    #[test]
    fn office_container_keeps_binary_parts_and_geometry() -> Result<()> {
        for ext in ["docx", "pptx"] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let main = if ext == "docx" {
                "word/document.xml"
            } else {
                "ppt/presentation.xml"
            };
            let content = if ext == "docx" {
                "word/header1.xml"
            } else {
                "ppt/slides/slide1.xml"
            };
            let xml=br#"<a:p xmlns:a="a"><a:pPr/><a:r><a:rPr b="1"/><a:t>Hello world</a:t></a:r></a:p>"#;
            for (name, data) in [
                (main, b"<root/>".as_slice()),
                (content, xml.as_slice()),
                ("media/image.png", b"untouched binary image".as_slice()),
                ("styles.xml", b"original geometry and styles".as_slice()),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())?;
                zip.write_all(data)?;
            }
            let (part, kind) = if ext == "docx" {
                ("word/document.xml", "wordprocessingml.document")
            } else {
                ("ppt/presentation.xml", "presentationml.presentation")
            };
            zip.start_file(
                "[Content_Types].xml",
                zip::write::SimpleFileOptions::default(),
            )?;
            write!(
                zip,
                "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.{kind}.main+xml\"/></Types>"
            )?;
            let input = zip.finish()?.into_inner();
            let output = translate(input, ext, |s| {
                assert_eq!(s, "Hello world");
                Ok("Hallo Welt".into())
            })?;
            let mut zip = zip::ZipArchive::new(Cursor::new(output))?;
            let mut text = String::new();
            zip.by_name(content)?.read_to_string(&mut text)?;
            assert!(text.contains("Hallo Welt"));
            assert!(text.contains("b=\"1\""));
            let mut binary = Vec::new();
            zip.by_name("media/image.png")?.read_to_end(&mut binary)?;
            assert_eq!(binary, b"untouched binary image");
            let mut styles = String::new();
            zip.by_name("styles.xml")?.read_to_string(&mut styles)?;
            assert_eq!(styles, "original geometry and styles");
        }
        Ok(())
    }
    #[test]
    fn office_runs_entities_and_layout_are_preserved() {
        let source = br#"<w:document xmlns:w="w"><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Hello &amp; </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>world</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12000"/></w:sectPr></w:document>"#;
        let (out, n) = xml(source, &mut |s| {
            assert_eq!(s, "Hello & world");
            Ok("Hallo & Welt".into())
        })
        .unwrap();
        assert_eq!(n, 2);
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("<w:b/>"));
        assert!(out.contains("w:w=\"12000\""));
        assert!(out.contains("xml:space=\"preserve\""));
        let mut actual = String::new();
        xml(out.as_bytes(), &mut |s| {
            actual = s.into();
            Ok(s.into())
        })
        .unwrap();
        assert_eq!(actual, "Hallo & Welt");
    }
    #[test]
    fn rejects_dtd_and_wrong_container() {
        assert!(xml(b"<!DOCTYPE a><a/>", &mut |_| unreachable!()).is_err());
        assert!(translate(b"not zip".to_vec(), "docx", |_| unreachable!()).is_err());
    }
    #[test]
    fn preserves_unicode_without_loss() {
        let (output, _) = xml(
            b"<w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world</w:t></w:r></w:p>",
            &mut |s| {
                assert_eq!(s, "Hello world");
                Ok("こんにちは 世界".into())
            },
        )
        .unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("こんにちは 世界")
        );
    }
}

#[cfg(test)]
mod security_regressions {
    use super::*;
    fn package(ext: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let (main, kind) = if ext == "docx" {
            ("word/document.xml", "wordprocessingml.document")
        } else {
            ("ppt/presentation.xml", "presentationml.presentation")
        };
        let types = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/{main}" ContentType="application/vnd.openxmlformats-officedocument.{kind}.main+xml"/></Types>"#
        );
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:document>"#;
        for (name, data) in [
            ("[Content_Types].xml", types.as_bytes()),
            (main, xml.as_bytes()),
        ]
        .into_iter()
        .chain(extra.iter().copied())
        {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn rejects_unsafe_office_parts_and_paths_before_translating() {
        for name in [
            "word/vbaProject.bin",
            "WORD/VBAPROJECT.BIN",
            "word/activeX/activeX1.xml",
            "word/embeddings/oleObject1.bin",
            "customUI/customUI.xml",
            "word/webExtensions/webExtension1.xml",
            "../outside.txt",
            "/absolute.txt",
            "C:/outside.txt",
            "word\\evil.xml",
            "word/../alias.xml",
            "word//alias.xml",
            "word/%2e%2e/alias.xml",
            "word/%2Falias.xml",
            "WORD/DOCUMENT.XML",
            "word/document.xml/",
        ] {
            let mut calls = 0;
            let result = translate(package("docx", &[(name, b"inert fixture")]), "docx", |_| {
                calls += 1;
                Ok("Hallo".into())
            });
            assert!(result.is_err(), "accepted {name}");
            assert_eq!(calls, 0, "sent text before rejecting {name}");
        }
    }
    #[test]
    fn rejects_dangerous_relationships_and_wrong_main_types_before_translating() {
        for relationship in [
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate" TargetMode="External" Target="https://example.invalid/a.dotm"/>"#,
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="embeddings/a.bin"/>"#,
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="file:///etc/passwd"/>"#,
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="javascript:alert(1)"/>"#,
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/attached&#84;emplate" Target="template.dotx"/>"#,
            r#"<Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../../../../outside.png"/>"#,
        ] {
            let rels = format!(
                r#"<p:Relationships xmlns:p="http://schemas.openxmlformats.org/package/2006/relationships" xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationship}</p:Relationships>"#
            );
            let mut calls = 0;
            assert!(
                translate(
                    package("docx", &[("word/_rels/document.xml.rels", rels.as_bytes())]),
                    "docx",
                    |_| {
                        calls += 1;
                        Ok("Hallo".into())
                    }
                )
                .is_err(),
                "accepted {relationship}"
            );
            assert_eq!(calls, 0);
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(package("pptx", &[]))).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..zip.len() {
            let file = zip.by_index(i).unwrap();
            out.raw_copy_file(file).unwrap();
        }
        // The PPTX main part must not pass as a DOCX container.
        assert!(
            translate(
                out.finish().unwrap().into_inner(),
                "docx",
                |_| unreachable!()
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_dde_and_imported_html_before_translating() {
        let xml = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>DDE</w:instrText></w:r><w:r><w:instrText>AUTO fixture inert</w:instrText><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>Hello</w:t></w:r></w:p></w:document>"#;
        let mut archive = zip::ZipArchive::new(Cursor::new(package("docx", &[]))).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..archive.len() {
            let file = archive.by_index(i).unwrap();
            if file.name().unwrap() == "word/document.xml" {
                out.start_file(
                    "word/document.xml",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                out.write_all(xml).unwrap();
            } else {
                out.raw_copy_file(file).unwrap();
            }
        }
        let mut calls = 0;
        assert!(
            translate(out.finish().unwrap().into_inner(), "docx", |_| {
                calls += 1;
                Ok("Hallo".into())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk" Target="import.html"/></Relationships>"#;
        assert!(
            translate(
                package("docx", &[("word/_rels/document.xml.rels", rels)]),
                "docx",
                |_| unreachable!()
            )
            .is_err()
        );
    }
    fn replace_part(input: Vec<u8>, part: &str, bytes: &[u8]) -> Vec<u8> {
        let mut archive = zip::ZipArchive::new(Cursor::new(input)).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..archive.len() {
            let file = archive.by_index(i).unwrap();
            if file.name().unwrap() == part {
                out.start_file(part, zip::write::SimpleFileOptions::default())
                    .unwrap();
                out.write_all(bytes).unwrap();
            } else {
                out.raw_copy_file(file).unwrap();
            }
        }
        out.finish().unwrap().into_inner()
    }
    #[test]
    fn rejects_field_instructions_in_relocated_word_parts() {
        let header=br#"<x:hdr xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:fldSimple x:instr="DDEAUTO inert fixture"/></x:hdr>"#;
        let rels=br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="h" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="../custom/header.dat"/></Relationships>"#;
        let types=br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/custom/header.dat" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#;
        let input = replace_part(
            package(
                "docx",
                &[
                    ("custom/header.dat", header),
                    ("word/_rels/document.xml.rels", rels),
                ],
            ),
            "[Content_Types].xml",
            types,
        );
        let mut calls = 0;
        assert!(
            translate(input, "docx", |_| {
                calls += 1;
                Ok("Hallo".into())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
    }
    #[test]
    fn preserves_bookmark_fields_and_page_numbers() {
        let fields=br#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:fldSimple w:instr='HYPERLINK \l "_Toc123"'/><w:fldSimple w:instr='HYPERLINK "https://example.invalid/" \o "Normal link"'/><w:r><w:fldChar w:fldCharType="begin"/><w:instrText> PAGE </w:instrText><w:fldChar w:fldCharType="separate"/><w:t>1</w:t><w:fldChar w:fldCharType="end"/></w:r></w:hdr>"#;
        let output = translate(
            package("docx", &[("word/header1.xml", fields)]),
            "docx",
            |_| Ok("Hallo".into()),
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(output)).unwrap();
        let mut actual = Vec::new();
        archive
            .by_name("word/header1.xml")
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, fields);
    }
    #[test]
    fn preserves_safe_hyperlinks_images_and_formatting() {
        let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="web" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="https://example.invalid/?a=1&amp;b=2"/><Relationship Id="mail" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="mailto:fixture@example.invalid"/><Relationship Id="img" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#;
        let output = translate(
            package(
                "docx",
                &[
                    ("word/_rels/document.xml.rels", rels),
                    ("word/media/image1.png", b"inert image fixture"),
                ],
            ),
            "docx",
            |_| Ok("Hallo".into()),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(output)).unwrap();
        let mut actual = Vec::new();
        zip.by_name("word/_rels/document.xml.rels")
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, rels);
        actual.clear();
        zip.by_name("word/media/image1.png")
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, b"inert image fixture");
    }
}
