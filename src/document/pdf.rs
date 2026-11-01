use anyhow::{Context, Result, ensure};
use pdfium_render::prelude::*;
use std::path::PathBuf;
mod layout;
pub fn library() -> Result<&'static Pdfium> {
    crate::embedded_pdfium::library()
}

fn font_path(family: &str, bold: bool, italic: bool) -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
        .join("Fonts");
    #[cfg(not(target_os = "windows"))]
    let root = {
        let bundled = std::env::current_exe().ok().and_then(|p| {
            p.parent()?
                .parent()
                .map(|dir| dir.join("share/fonts/truetype/dejavu"))
        });
        bundled
            .into_iter()
            .chain([
                PathBuf::from("/usr/share/fonts/truetype/dejavu"),
                PathBuf::from("/usr/share/fonts/TTF"),
            ])
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("/usr/share/fonts/truetype/dejavu"))
    };
    #[cfg(target_os = "windows")]
    let name = match (family.to_lowercase().as_str(), bold, italic) {
        (f, b, i) if f.contains("arial") || f.contains("helvetica") => match (b, i) {
            (true, true) => "arialbi.ttf",
            (true, false) => "arialbd.ttf",
            (false, true) => "ariali.ttf",
            _ => "arial.ttf",
        },
        (f, b, i) if f.contains("times") => match (b, i) {
            (true, true) => "timesbi.ttf",
            (true, false) => "timesbd.ttf",
            (false, true) => "timesi.ttf",
            _ => "times.ttf",
        },
        (f, b, i) if f.contains("calibri") => match (b, i) {
            (true, true) => "calibriz.ttf",
            (true, false) => "calibrib.ttf",
            (false, true) => "calibrii.ttf",
            _ => "calibri.ttf",
        },
        (_, true, true) => "segoeuiz.ttf",
        (_, true, false) => "segoeuib.ttf",
        (_, false, true) => "segoeuii.ttf",
        _ => "segoeui.ttf",
    };
    #[cfg(not(target_os = "windows"))]
    let _ = family;
    #[cfg(not(target_os = "windows"))]
    let name = match (bold, italic) {
        (true, true) => "DejaVuSans-BoldOblique.ttf",
        (true, false) => "DejaVuSans-Bold.ttf",
        (false, true) => "DejaVuSans-Oblique.ttf",
        _ => "DejaVuSans.ttf",
    };
    let path = root.join(name);
    ensure!(
        path.is_file(),
        "PDF-Ersatzschrift fehlt: {}",
        path.display()
    );
    Ok(path)
}

#[cfg(test)]
pub fn translate(
    bytes: Vec<u8>,
    translator: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<u8>> {
    translate_cancellable(bytes, translator, &mut || Ok(()))
}
pub fn translate_cancellable(
    bytes: Vec<u8>,
    mut translator: impl FnMut(&str) -> Result<String>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    check()?;
    ensure!(bytes.starts_with(b"%PDF-"), "Ungültige PDF-Datei.");
    let pdfium = library()?;
    let mut document = pdfium
        .load_pdf_from_byte_vec(bytes, None)
        .context("PDF konnte nicht geöffnet werden (ggf. verschlüsselt).")?;
    ensure!(
        document.signatures().is_empty(),
        "Signierte PDFs werden nicht verändert."
    );
    ensure!(document.pages().len() <= 500, "Maximal 500 PDF-Seiten.");
    // Embed full Unicode TrueType fonts rather than reuse an incomplete source subset.
    let mut paths = std::collections::HashSet::new();
    for page in document.pages().iter() {
        check()?;
        for object in page.objects().iter() {
            check()?;
            if let Some(text) = object.as_text_object() {
                let font = text.font();
                paths.insert(font_path(
                    &font.name(),
                    font.name().to_lowercase().contains("bold"),
                    font.is_italic(),
                )?);
            }
        }
    }
    let mut fonts = std::collections::HashMap::new();
    for path in paths {
        check()?;
        let data = std::fs::read(&path)?;
        let token = document.fonts_mut().load_true_type_from_file(&path, true)?;
        fonts.insert(path, (token, data));
    }
    let mut count = 0;
    let mut layout_work = 0usize;
    let mut layout_spans = 0usize;
    for mut page in document.pages().iter() {
        check()?;
        // Do not silently return a partly translated page containing Form XObjects.
        ensure!(
            !page
                .objects()
                .iter()
                .any(|o| o.as_x_object_form_object().is_some()),
            "PDF enthält verschachtelte Form-Objekte; verlustfreie Bearbeitung wird noch nicht unterstützt."
        );
        let regions: Vec<_> = page
            .objects()
            .iter()
            .enumerate()
            .filter(|(_, object)| object.as_text_object().is_some())
            .map(|(index, object)| object.bounds().map(|bounds| (index, bounds.to_rect())))
            .collect::<std::result::Result<_, _>>()?;
        let content_right = regions
            .iter()
            .map(|(_, rect)| rect.right().value)
            .fold(0f32, f32::max);
        let indices: Vec<_> = page
            .objects()
            .iter()
            .enumerate()
            .filter_map(|(i, o)| {
                o.as_text_object()
                    .filter(|t| t.text().chars().any(char::is_alphabetic))
                    .map(|_| i)
            })
            .collect();
        ensure!(
            !indices.is_empty()
                || page.objects().iter().any(|o| o.as_text_object().is_some())
                || page.text()?.all().trim().is_empty(),
            "PDF-Text ist nicht als bearbeitbares Textobjekt verfügbar."
        );
        ensure!(
            !indices.is_empty() || !page.objects().iter().any(|o| o.as_image_object().is_some()),
            "Eine PDF-Seite enthält nur ein Bild. Layout-OCR für gescannte PDFs wird noch nicht unterstützt."
        );
        let mut plans = layout::plan_page(&page, &indices, &mut layout_work, check)?;
        layout_spans += plans.values().map(Vec::len).sum::<usize>();
        ensure!(layout_spans <= 100_000, "Dokument zu groß.");
        for index in indices.into_iter().rev() {
            check()?;
            let original = page.objects().get(index)?;
            let text = original.as_text_object().unwrap();
            let source = text.text();
            ensure!(
                source.chars().count() <= 5000,
                "Ein PDF-Textobjekt überschreitet 5.000 Zeichen."
            );
            let mut spans = plans.remove(&index).unwrap();
            let matrix = text.matrix()?;
            let size = text.unscaled_font_size();
            if spans.len() > 1
                && matrix.b().abs() < 0.0001
                && matrix.c().abs() < 0.0001
                && matrix.a() > 0.
                && matrix.d() > 0.
            {
                let last = spans.last_mut().unwrap();
                let (left, baseline) =
                    matrix.apply_to_points(PdfPoints::new(last.x), PdfPoints::new(last.y));
                let tolerance = size.value * matrix.d() * 0.4;
                let right = regions
                    .iter()
                    .filter(|(other, rect)| {
                        *other != index
                            && rect.left().value > left.value + 0.1
                            && rect.bottom().value <= baseline.value + tolerance
                            && rect.top().value >= baseline.value - tolerance
                    })
                    .map(|(_, rect)| rect.left().value)
                    .fold(content_right, f32::min);
                let (right, _) = matrix
                    .invert()
                    .apply_to_points(PdfPoints::new(right), baseline);
                last.available = (right.value - last.x - size.value * 0.3).max(last.width);
            }
            let table_codes = spans.len() > 1
                && spans
                    .iter()
                    .any(|span| span.text.chars().any(|c| c.is_ascii_digit()));
            if !spans
                .iter()
                .any(|span| layout::translatable(&span.text, table_codes))
            {
                continue;
            }
            let font = text.font();
            let bold = font.name().to_lowercase().contains("bold");
            let (token, data) = &fonts[&font_path(&font.name(), bold, font.is_italic())?];
            let face = ttf_parser::Face::parse(data, 0)
                .map_err(|_| anyhow::anyhow!("PDF-Schrift ungültig."))?;
            let mut replacements = Vec::new();
            let mut changed = false;
            for span in spans {
                check()?;
                let translate_span = layout::translatable(&span.text, table_codes);
                let translated = if translate_span {
                    translator(&span.text)?
                } else {
                    span.text.clone()
                };
                ensure!(!translated.trim().is_empty(), "Leere PDF-Übersetzung.");
                ensure!(
                    translated.len() <= super::MAX_TRANSLATION_BYTES,
                    "Dokument zu groß."
                );
                let changed_span = translated != span.text;
                changed |= changed_span;
                if changed_span {
                    ensure!(
                        translated
                            .chars()
                            .all(|c| c.is_whitespace() || face.glyph_index(c).is_some()),
                        "Die lokale PDF-Schrift enthält nicht alle Zeichen der Zielsprache."
                    );
                }
                let selected_font = if changed_span { *token } else { font.token() };
                let mut replacement =
                    PdfPageTextObject::new(&document, &translated, selected_font, size)?;
                replacement.set_fill_color(text.fill_color()?)?;
                replacement.set_render_mode(text.render_mode())?;
                replacement.set_stroke_color(text.stroke_color()?)?;
                replacement.set_stroke_width(text.stroke_width()?)?;
                let new_width = replacement.bounds()?.width().value.max(0.1);
                let fit = if changed_span {
                    (span.available / new_width).min(1.0)
                } else {
                    1.
                };
                ensure!(
                    fit >= 0.5,
                    "Die Übersetzung passt nicht lesbar in das ursprüngliche PDF-Textfeld."
                );
                let (x, y) = matrix.apply_to_points(PdfPoints::new(span.x), PdfPoints::new(span.y));
                replacement.reset_matrix(PdfMatrix::new(
                    matrix.a() * fit,
                    matrix.b() * fit,
                    matrix.c(),
                    matrix.d(),
                    x.value,
                    y.value,
                ))?;
                replacements.push(replacement);
                if translate_span {
                    count += 1;
                }
            }
            if !changed {
                continue;
            }
            drop(font);
            drop(original);
            page.objects_mut().remove_object_at_index(index)?;
            // Keep paint order: appending would move text above later graphics.
            for replacement in replacements.into_iter().rev() {
                page.objects_mut()
                    .insert_object_at_index(index, replacement.into())?;
            }
        }
    }
    ensure!(
        count > 0,
        "PDF enthält keinen bearbeitbaren Text. Gescannte PDFs benötigen Layout-OCR und werden noch nicht unterstützt."
    );
    Ok(document.save_to_bytes()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires embedded PDFium and replacement fonts"]
    fn subset_cid_fonts_keep_untranslated_numeric_rows_visible() -> Result<()> {
        let pdfium = library()?;
        let input = include_bytes!("../../tests/fixtures/pdf-positioned-cid-cells.pdf").to_vec();
        let output = translate(input.clone(), |source| {
            Ok(match source {
                "Green tea" => "Vihreä tee",
                "Product" => "Tuote",
                _ => source,
            }
            .to_owned())
        })?;
        fn ink(pdfium: &Pdfium, bytes: Vec<u8>) -> Result<usize> {
            let doc = pdfium.load_pdf_from_byte_vec(bytes, None)?;
            let page = doc.pages().get(0)?;
            let bitmap =
                page.render_with_config(&PdfRenderConfig::new().set_target_size(595, 842))?;
            let pixels = bitmap.as_raw_bytes();
            let mut count = 0;
            for y in 158..180 {
                for x in 295..490 {
                    let at = (y * bitmap.width() as usize + x) * 4;
                    if pixels[at..at + 3].iter().all(|v| *v < 180) {
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        let before = ink(pdfium, input)?;
        let after = ink(pdfium, output)?;
        assert!(before > 40);
        assert!(
            after * 10 >= before * 8,
            "unchanged numeric row disappeared: {before} → {after}"
        );
        Ok(())
    }
    #[test]
    #[ignore = "Requires embedded PDFium and replacement fonts"]
    fn positioned_table_cells_keep_columns_and_numeric_values() -> Result<()> {
        let pdfium = library()?;
        let input = include_bytes!("../../tests/fixtures/pdf-positioned-cells.pdf").to_vec();
        let mut calls = Vec::new();
        let output = translate(input, |source| {
            calls.push(source.to_string());
            ensure!(
                !source.contains("12.50"),
                "numeric cells must not be sent for translation"
            );
            Ok(match source {
                "Product" => "Tuote",
                "Quantity" => "Määrä",
                "VAT" => "ALV",
                "Unit price" => "Yksikköhinta",
                "Total" => "Yhteensä",
                "Green tea" => "Vihreä tee",
                _ => anyhow::bail!("Unexpected combined cell: {source}"),
            }
            .into())
        })?;
        assert_eq!(calls.len(), 6);
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        let text = page.text()?;
        for (value, x, right) in [
            ("2", 300., 340.),
            ("B", 350., 400.),
            ("12.50 EUR", 420., 500.),
            ("25.00 EUR", 510., 580.),
        ] {
            let region = PdfRect::new(
                PdfPoints::new(690.),
                PdfPoints::new(x - 1.),
                PdfPoints::new(710.),
                PdfPoints::new(right),
            );
            assert_eq!(
                text.inside_rect(region).trim(),
                value,
                "cell {value} moved into another column"
            );
            assert!(
                text.chars()
                    .iter()
                    .any(|c| c.unicode_char() == value.chars().next()
                        && c.origin_x().is_ok_and(|v| (v.value - x).abs() < 0.1)
                        && c.origin_y().is_ok_and(|v| (v.value - 700.).abs() < 0.1)),
                "cell {value} origin moved"
            );
        }
        assert!(
            text.all()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("Vihreä tee"),
            "Translated cell text: {:?}",
            text.all()
        );
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement fonts"]
    fn overlapping_objects_with_identical_matrices_keep_separate_text() -> Result<()> {
        let pdfium = library()?;
        let mut doc = pdfium.create_new_pdf()?;
        let font = doc.fonts_mut().helvetica();
        {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            for value in ["Hello", "World"] {
                page.objects_mut().create_text_object(
                    PdfPoints::new(50.),
                    PdfPoints::new(700.),
                    value,
                    font,
                    PdfPoints::new(18.),
                )?;
            }
        }
        let mut calls = Vec::new();
        let output = translate(doc.save_to_bytes()?, |source| {
            calls.push(source.to_string());
            Ok(match source {
                "Hello" => "Hallo",
                "World" => "Welt",
                _ => anyhow::bail!("Objects merged"),
            }
            .into())
        })?;
        calls.sort();
        assert_eq!(calls, ["Hello", "World"]);
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        // PDFium's page extractor may suppress overlapping output glyphs. Check
        // independent ownership before translation and separate objects afterward.
        assert_eq!(page.objects().len(), 2);
        for object in page.objects().iter() {
            let matrix = object.as_text_object().unwrap().matrix()?;
            assert!((matrix.e() - 50.).abs() < 0.1);
            assert!((matrix.f() - 700.).abs() < 0.1);
        }
        Ok(())
    }

    #[test]
    #[ignore = "Local private PDF inspection; requires DEEPL_PDF_INSPECT and DEEPL_PDF_JSON"]
    fn inspect_private_pdf_objects() -> Result<()> {
        let bytes = std::fs::read(std::env::var("DEEPL_PDF_INSPECT")?)?;
        let pdfium = library()?;
        let doc = pdfium.load_pdf_from_byte_vec(bytes, None)?;
        let mut pages = Vec::new();
        for page in doc.pages().iter() {
            let text_page = page.text()?;
            let mut objects = Vec::new();
            for (index, object) in page.objects().iter().enumerate() {
                if let Some(text) = object.as_text_object() {
                    let matrix = text.matrix()?;
                    let mut chars = Vec::new();
                    for c in text.chars(&text_page)?.iter() {
                        let (x, y) = matrix
                            .invert()
                            .apply_to_points(c.origin_x()?, c.origin_y()?);
                        let bounds = c.loose_bounds()?.transform(matrix.invert());
                        chars.push(serde_json::json!({"c":c.unicode_char(),"x":x.value,"y":y.value,"right":bounds.right().value,"generated":c.is_generated()?}));
                    }
                    let color = text.fill_color()?;
                    objects.push(serde_json::json!({"index":index,"text":text.text(),"matrix":[matrix.a(),matrix.b(),matrix.c(),matrix.d(),matrix.e(),matrix.f()],"font":text.font().name(),"size":text.unscaled_font_size().value,"mode":format!("{:?}",text.render_mode()),"color":[color.red(),color.green(),color.blue(),color.alpha()],"chars":chars}));
                }
            }
            pages.push(objects);
        }
        std::fs::write(
            std::env::var("DEEPL_PDF_JSON")?,
            serde_json::to_vec_pretty(&pages)?,
        )?;
        Ok(())
    }

    #[test]
    #[ignore = "Local PDF replay; requires explicit DEEPL_PDF_INPUT/OUTPUT/CALLS; never uses a network provider"]
    fn replay_private_pdf_with_local_translations() -> Result<()> {
        let bytes = std::fs::read(std::env::var("DEEPL_PDF_INPUT")?)?;
        let translations: Option<std::collections::HashMap<String, String>> =
            std::env::var("DEEPL_PDF_TRANSLATIONS")
                .ok()
                .map(|path| -> Result<_> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
                .transpose()?;
        let mut calls = Vec::new();
        let result = translate(bytes, |source| {
            calls.push(source.to_owned());
            match &translations {
                Some(values) => values
                    .get(source)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Missing local translation fixture")),
                None => Ok(source.to_owned()),
            }
        });
        std::fs::write(
            std::env::var("DEEPL_PDF_CALLS")?,
            serde_json::to_vec_pretty(&calls)?,
        )?;
        std::fs::write(std::env::var("DEEPL_PDF_OUTPUT")?, result?)?;
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement system fonts"]
    fn pdf_translation_preserves_text_and_graphics_paint_order() -> Result<()> {
        let pdfium = library()?;
        let mut doc = pdfium.create_new_pdf()?;
        let font = doc.fonts_mut().helvetica();
        {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            page.objects_mut().create_text_object(
                PdfPoints::new(50.),
                PdfPoints::new(700.),
                "First",
                font,
                PdfPoints::new(18.),
            )?;
            page.objects_mut().create_path_object_rect(
                PdfRect::new(
                    PdfPoints::new(690.),
                    PdfPoints::new(40.),
                    PdfPoints::new(730.),
                    PdfPoints::new(140.),
                ),
                None,
                None,
                Some(PdfColor::WHITE),
            )?;
            page.objects_mut().create_text_object(
                PdfPoints::new(50.),
                PdfPoints::new(650.),
                "Second",
                font,
                PdfPoints::new(18.),
            )?;
        }
        let output = translate(doc.save_to_bytes()?, |text| Ok(format!("T{text}")))?;
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        let objects: Vec<_> = page
            .objects()
            .iter()
            .map(|object| {
                object
                    .as_text_object()
                    .map(|text| text.text())
                    .unwrap_or_else(|| "[path]".into())
            })
            .collect();
        assert_eq!(objects, ["TFirst", "[path]", "TSecond"]);
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement system fonts"]
    fn pdf_translation_preserves_invisible_and_stroked_text() -> Result<()> {
        let pdfium = library()?;
        let mut doc = pdfium.create_new_pdf()?;
        let font = doc.fonts_mut().helvetica();
        {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            for (y, mode) in [
                (700., PdfPageTextRenderMode::Invisible),
                (650., PdfPageTextRenderMode::StrokedUnfilled),
            ] {
                let mut text = page.objects_mut().create_text_object(
                    PdfPoints::new(50.),
                    PdfPoints::new(y),
                    "Hello",
                    font,
                    PdfPoints::new(18.),
                )?;
                text.as_text_object_mut().unwrap().set_render_mode(mode)?;
                text.set_stroke_color(PdfColor::RED)?;
                text.set_stroke_width(PdfPoints::new(2.))?;
            }
            page.regenerate_content()?;
        }
        let output = translate(doc.save_to_bytes()?, |_| Ok("Hallo".to_string()))?;
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        let hidden = page.objects().get(0)?;
        assert_eq!(
            hidden.as_text_object().unwrap().render_mode(),
            PdfPageTextRenderMode::Invisible
        );
        let stroked = page.objects().get(1)?;
        assert_eq!(
            stroked.as_text_object().unwrap().render_mode(),
            PdfPageTextRenderMode::StrokedUnfilled
        );
        let color = stroked.stroke_color()?;
        assert_eq!((color.red(), color.green(), color.blue()), (255, 0, 0));
        assert!((stroked.stroke_width()?.value - 2.).abs() < 0.01);
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement system fonts"]
    fn pdf_translation_keeps_numeric_pages_without_rejecting_the_document() -> Result<()> {
        let pdfium = library()?;
        let mut doc = pdfium.create_new_pdf()?;
        let font = doc.fonts_mut().helvetica();
        for source in ["2026", "Hello"] {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            page.objects_mut().create_text_object(
                PdfPoints::new(50.),
                PdfPoints::new(700.),
                source,
                font,
                PdfPoints::new(18.),
            )?;
        }
        let output = translate(doc.save_to_bytes()?, |text| {
            assert_eq!(text, "Hello");
            Ok("Hallo".into())
        })?;
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        assert_eq!(read.pages().len(), 2);
        assert_eq!(read.pages().get(0)?.text()?.all().trim(), "2026");
        assert_eq!(read.pages().get(1)?.text()?.all().trim(), "Hallo");
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement system fonts"]
    fn rotated_pdf_text_rejects_a_translation_that_exceeds_its_baseline() -> Result<()> {
        let pdfium = library()?;
        let mut doc = pdfium.create_new_pdf()?;
        let font = doc.fonts_mut().helvetica();
        {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            let mut text = page.objects_mut().create_text_object(
                PdfPoints::ZERO,
                PdfPoints::ZERO,
                "Hello",
                font,
                PdfPoints::new(18.),
            )?;
            text.reset_matrix(PdfMatrix::new(0., 1., -1., 0., 100., 100.))?;
            page.regenerate_content()?;
        }
        let bytes = doc.save_to_bytes()?;
        let output = translate(bytes.clone(), |_| Ok("Hallo".to_string()))?;
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        let object = page.objects().get(0)?;
        let matrix = object.matrix()?;
        assert_eq!(object.as_text_object().unwrap().text(), "Hallo");
        assert!(matrix.a().abs() < 0.001 && matrix.b() > 0.5);
        assert!((matrix.c() + 1.).abs() < 0.001 && matrix.d().abs() < 0.001);
        let result = translate(bytes, |_| Ok("Hello Hello Hello Hello Hello Hello".into()));
        assert!(
            result.is_err(),
            "rotated text must obey the same readability limit as horizontal text"
        );
        assert!(result.unwrap_err().to_string().contains("nicht lesbar"));
        Ok(())
    }

    #[test]
    #[ignore = "Requires embedded PDFium and replacement system fonts"]
    fn pdf_roundtrip_preserves_page_and_graphics_and_translates_unicode() -> Result<()> {
        let pdfium = library()?;
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
            page.objects_mut().create_path_object_rect(
                PdfRect::new(
                    PdfPoints::new(100.),
                    PdfPoints::new(50.),
                    PdfPoints::new(150.),
                    PdfPoints::new(300.),
                ),
                Some(PdfColor::BLUE),
                Some(PdfPoints::new(1.)),
                Some(PdfColor::GREEN),
            )?;
        }
        let input = doc.save_to_bytes()?;
        let output = translate(input, |text| {
            assert_eq!(text, "Hello world");
            Ok("Grüße světe".into())
        })?;
        let read = pdfium.load_pdf_from_byte_vec(output, None)?;
        let page = read.pages().get(0)?;
        assert_eq!(page.width(), PdfPagePaperSize::a4().width());
        assert!(
            page.text()?.all().split_whitespace().collect::<Vec<_>>() == ["Grüße", "světe"],
            "Actual text: {:?}",
            page.text()?.all()
        );
        assert_eq!(
            page.objects()
                .iter()
                .filter(|o| o.as_path_object().is_some())
                .count(),
            1
        );
        let text = page
            .objects()
            .iter()
            .find(|o| o.as_text_object().is_some())
            .unwrap();
        let m = text.matrix()?;
        assert!((m.e() - 50.).abs() < 0.01);
        assert!((m.f() - 700.).abs() < 0.01);
        Ok(())
    }
}
