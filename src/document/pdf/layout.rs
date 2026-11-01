//! Recover independently positioned runs without flattening PDF table columns.
use anyhow::{Result, ensure};
use pdfium_render::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub(super) struct Span {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub available: f32,
}

pub(super) fn spans(object: &PdfPageTextObject<'_>, page: &PdfPageText<'_>) -> Result<Vec<Span>> {
    let chars = page.chars_for_object(object)?;
    build_spans(object, chars.iter().map(Ok))
}

fn matrix_key(matrix: PdfMatrix) -> [u32; 6] {
    [
        matrix.a(),
        matrix.b(),
        matrix.c(),
        matrix.d(),
        matrix.e(),
        matrix.f(),
    ]
    .map(f32::to_bits)
}

pub(super) fn plan_page(
    page: &PdfPage<'_>,
    indices: &[usize],
    work: &mut usize,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<HashMap<usize, Vec<Span>>> {
    let text = page.text()?;
    let all = text.chars();
    *work = work.saturating_add(all.len());
    ensure!(*work <= 20_000_000, "Dokument zu groß.");
    let wanted: HashSet<_> = indices.iter().copied().collect();
    let mut owners: HashMap<[u32; 6], Option<usize>> = HashMap::new();
    for (index, object) in page.objects().iter().enumerate() {
        if let Some(object) = object.as_text_object() {
            owners
                .entry(matrix_key(object.matrix()?))
                .and_modify(|owner| *owner = None)
                .or_insert(Some(index));
        }
    }
    let mut grouped: HashMap<usize, Vec<usize>> = HashMap::new();
    for (n, character) in all.iter().enumerate() {
        if n % 256 == 0 {
            check()?;
        }
        if character.is_generated()? {
            continue;
        }
        let owner = match character.text_object() {
            Ok(owner) => owner,
            Err(_) if character.unicode_char().is_some_and(char::is_whitespace) => continue,
            Err(error) => return Err(error.into()),
        };
        if let Some(Some(index)) = owners.get(&matrix_key(owner.matrix()?))
            && wanted.contains(index)
        {
            grouped.entry(*index).or_default().push(character.index());
        }
    }
    let mut result = HashMap::new();
    for &index in indices {
        check()?;
        let object = page.objects().get(index)?;
        let object = object.as_text_object().unwrap();
        let values = if owners[&matrix_key(object.matrix()?)].is_some() {
            let selected = grouped.remove(&index).unwrap_or_default();
            build_spans(object, selected.into_iter().map(|i| all.get(i)))?
        } else {
            // Overlapping objects may share a matrix; use exact ownership there.
            *work = work.saturating_add(all.len());
            ensure!(*work <= 20_000_000, "Dokument zu groß.");
            spans(object, &text)?
        };
        result.insert(index, values);
    }
    Ok(result)
}

fn build_spans<'a>(
    object: &PdfPageTextObject<'_>,
    chars: impl Iterator<Item = std::result::Result<PdfPageTextChar<'a>, PdfiumError>>,
) -> Result<Vec<Span>> {
    let matrix = object.matrix()?;
    ensure!(
        matrix.determinant().is_finite() && matrix.determinant().abs() > 0.000001,
        "PDF-Text ist nicht als bearbeitbares Textobjekt verfügbar."
    );
    let inverse = matrix.invert();
    let size = object.unscaled_font_size().value.abs().max(1.);
    let mut result: Vec<Span> = Vec::new();
    let mut whitespace = String::new();
    for (n, character) in chars.enumerate() {
        ensure!(
            n < 10_000,
            "Ein PDF-Textobjekt überschreitet 5.000 Zeichen."
        );
        let character = character?;
        if character.is_generated()? {
            continue;
        }
        let Some(c) = character.unicode_char() else {
            anyhow::bail!("PDF-Text ist nicht als bearbeitbares Textobjekt verfügbar.");
        };
        if c.is_whitespace() {
            whitespace.push(c);
            continue;
        }
        ensure!(
            !c.is_control(),
            "PDF-Text ist nicht als bearbeitbares Textobjekt verfügbar."
        );
        let (x, y) = inverse.apply_to_points(character.origin_x()?, character.origin_y()?);
        let bounds = character.loose_bounds()?.transform(inverse);
        let (x, y, right) = (x.value, y.value, bounds.right().value);
        ensure!(
            [x, y, right].iter().all(|v| v.is_finite()),
            "Ungültige PDF-Datei."
        );
        let separate = result.last().is_none_or(|last| {
            (y - last.y).abs() > size * 0.4 || x - (last.x + last.width) > size * 0.9
        });
        if separate {
            result.push(Span {
                text: c.to_string(),
                x,
                y,
                width: (right - x).max(0.1),
                available: 0.,
            });
        } else {
            let last = result.last_mut().unwrap();
            if !whitespace.is_empty() {
                last.text.push_str(&whitespace);
            } else if x - (last.x + last.width) > size * 0.18 {
                last.text.push(' ');
            }
            last.text.push(c);
            last.width = last.width.max(right - last.x);
        }
        whitespace.clear();
    }
    for i in 0..result.len() {
        result[i].available = result[i].width;
        if i + 1 < result.len() && (result[i].y - result[i + 1].y).abs() < size * 0.4 {
            // Use the blank part of this cell, but never enter the next column.
            result[i].available = (result[i + 1].x - result[i].x - size * 0.4).max(result[i].width);
        }
    }
    ensure!(
        !result.is_empty(),
        "PDF-Text ist nicht als bearbeitbares Textobjekt verfügbar."
    );
    Ok(result)
}

pub(super) fn translatable(text: &str, table_codes: bool) -> bool {
    let letters: String = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return false;
    }
    // Tax codes, currency/measurement values and identifiers are data, not prose.
    if table_codes && letters.len() <= 2 && letters.chars().all(|c| c.is_ascii_uppercase()) {
        return false;
    }
    if text.contains('/') && letters.len() <= 2 && letters.chars().all(|c| c.is_ascii_uppercase()) {
        return false;
    }
    if text.chars().any(|c| c.is_ascii_digit()) {
        if letters.len() <= 2 && letters.chars().all(|c| c.is_ascii_uppercase()) {
            return false;
        }
        if letters.chars().all(|c| c.is_ascii_uppercase())
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ['-', '/', '_'].contains(&c))
        {
            return false;
        }
        if [
            "eur", "usd", "gbp", "chf", "jpy", "czk", "g", "kg", "mg", "l", "ml", "cl", "cm", "mm",
            "m", "st", "stk",
        ]
        .contains(&letters.to_lowercase().as_str())
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_cells_and_codes_are_preserved_but_headers_and_descriptions_translate() {
        for source in [
            "2",
            "B",
            "A/B",
            "A = 19 %",
            "12,50\u{a0}€",
            "12.50 EUR",
            "968g",
            "DE8123456789",
        ] {
            assert!(!translatable(source, true), "{source}");
        }
        for source in [
            "Product",
            "Quantity",
            "VAT",
            "Unit price",
            "Green tea 500g",
            "Steuersatz",
            "PFAND 0,25",
        ] {
            assert!(translatable(source, true), "{source}");
        }
        for source in ["I", "ON", "EU"] {
            assert!(translatable(source, false));
        }
    }
}
