pub mod config;
pub mod history;
pub mod placement;
pub mod popup;
pub mod quick_session;
pub mod window_size;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingStyle {
    #[default]
    Default,
    Simple,
    Business,
    Academic,
    Casual,
}
impl WritingStyle {
    pub fn value(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Simple => Some("simple"),
            Self::Business => Some("business"),
            Self::Academic => Some("academic"),
            Self::Casual => Some("casual"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslationRequest {
    pub text: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub style: WritingStyle,
}

impl TranslationRequest {
    pub fn validate_write(&self) -> Result<(), String> {
        self.validate()?;
        if !WRITE_LANGUAGES.contains(&self.target.as_str()) {
            return Err("Write unterstützt diese Sprache nicht.".into());
        }
        if self.style != WritingStyle::Default && !STYLE_LANGUAGES.contains(&self.target.as_str()) {
            return Err(
                "Schreibstile sind für diese Sprache nicht verfügbar. Bitte Standard wählen."
                    .into(),
            );
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("Bitte Text eingeben.".into());
        }
        if self.text.chars().count() > 5_000 {
            return Err("Maximal 5.000 Zeichen pro Anfrage.".into());
        }
        if !LANGUAGES.contains(&self.target.as_str()) || self.target == "AUTO" {
            return Err("Ungültige Zielsprache.".into());
        }
        if !LANGUAGES.contains(&self.source.as_str()) {
            return Err("Ungültige Quellsprache.".into());
        }
        Ok(())
    }
}

pub const STYLE_LANGUAGES: &[&str] = &[
    "DE", "EN", "EN-GB", "EN-US", "FR", "ES", "IT", "PT", "PT-BR", "PT-PT",
];

pub const WRITE_LANGUAGES: &[&str] = &[
    "DE", "EN", "EN-GB", "EN-US", "FR", "ES", "IT", "PT", "PT-BR", "PT-PT", "JA", "ZH", "KO",
];

pub const LANGUAGES: &[&str] = &[
    "AUTO", "DE", "EN", "EN-GB", "EN-US", "FR", "ES", "IT", "NL", "PL", "PT", "PT-BR", "PT-PT",
    "JA", "ZH", "KO", "RU", "UK", "CS", "DA", "EL", "FI", "HU", "ID", "NB", "RO", "SK", "SL", "SV",
    "TR", "BG", "ET", "LT", "LV", "AR",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Translation {
    pub text: String,
    pub detected_source: String,
}
