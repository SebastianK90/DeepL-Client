use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    #[default]
    ApiFree,
    ApiPro,
    Web,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    #[default]
    Translate,
    Write,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickModePolicy {
    #[default]
    RememberLast,
    AlwaysTranslate,
    AlwaysWrite,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickPositionPolicy {
    #[default]
    RememberLast,
    Centered,
    MouseCursor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: ProviderMode,
    pub source: String,
    pub target: String,
    pub theme: String,
    pub hotkey_ms: u64,
    pub history_enabled: bool,
    pub start_in_tray: bool,
    pub instance_id: uuid::Uuid,
    pub quick_mode_policy: QuickModePolicy,
    pub last_mode: WorkMode,
    pub write_language: String,
    #[serde(deserialize_with = "selected_style")]
    pub write_style: super::WritingStyle,
    pub ui_language: String,
    pub quick_position_policy: QuickPositionPolicy,
    pub quick_position: Option<SavedPosition>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            provider: ProviderMode::ApiFree,
            source: "AUTO".into(),
            target: "DE".into(),
            theme: "light".into(),
            hotkey_ms: 350,
            history_enabled: true,
            start_in_tray: true,
            instance_id: uuid::Uuid::new_v4(),
            quick_mode_policy: QuickModePolicy::RememberLast,
            last_mode: WorkMode::Translate,
            write_language: "DE".into(),
            write_style: super::WritingStyle::Simple,
            ui_language: "de".into(),
            quick_position_policy: QuickPositionPolicy::RememberLast,
            quick_position: None,
        }
    }
}
fn selected_style<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<super::WritingStyle, D::Error> {
    super::WritingStyle::deserialize(deserializer).map(|style| match style {
        super::WritingStyle::Default => super::WritingStyle::Simple,
        style => style,
    })
}
impl Config {
    pub fn initial_quick_mode(&self) -> WorkMode {
        match self.quick_mode_policy {
            QuickModePolicy::RememberLast => self.last_mode,
            QuickModePolicy::AlwaysTranslate => WorkMode::Translate,
            QuickModePolicy::AlwaysWrite => WorkMode::Write,
        }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            ["en", "de", "cs", "fr", "ja", "it"].contains(&self.ui_language.as_str()),
            "Ungültige Oberflächensprache."
        );
        anyhow::ensure!(
            super::WRITE_LANGUAGES.contains(&self.write_language.as_str()),
            "Diese Sprache wird von Write nicht unterstützt."
        );
        anyhow::ensure!(
            (300..=400).contains(&self.hotkey_ms),
            "Hotkey-Fenster muss zwischen 300 und 400 ms liegen."
        );
        anyhow::ensure!(
            ["light", "dark", "system"].contains(&self.theme.as_str()),
            "Unbekanntes Theme."
        );
        anyhow::ensure!(
            super::LANGUAGES.contains(&self.source.as_str()),
            "Ungültige Quellsprache."
        );
        anyhow::ensure!(
            self.target != "AUTO" && super::LANGUAGES.contains(&self.target.as_str()),
            "Ungültige Zielsprache."
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct Storage {
    pub config: PathBuf,
    pub history: PathBuf,
}
impl Storage {
    pub fn discover() -> Result<Self> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("DEEPL_CLIENT_TEST_DIR") {
            let path = PathBuf::from(path);
            return Ok(Self {
                config: path.join("config.toml"),
                history: path.join("history.json"),
            });
        }
        let dirs = ProjectDirs::from("dev", "Local", "DeepLClient")
            .context("Benutzerverzeichnis nicht verfügbar")?;
        Ok(Self {
            config: dirs.config_dir().join("config.toml"),
            history: dirs.data_local_dir().join("history.json"),
        })
    }
    pub fn load_config(&self) -> Result<Config> {
        match std::fs::read_to_string(&self.config) {
            Ok(text) => {
                let config: Config = toml::from_str(&text).context("config.toml ist ungültig")?;
                config.validate()?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Config::default();
                self.save_config(&config)?;
                Ok(config)
            }
            Err(e) => Err(e.into()),
        }
    }
    pub fn save_config(&self, config: &Config) -> Result<()> {
        config.validate()?;
        atomic_write(&self.config, toml::to_string_pretty(config)?.as_bytes())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Kein übergeordnetes Verzeichnis")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn key_entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        "dev.local.deepl-client",
        "deepl-api-key",
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_start_defaults_on_for_existing_configs_and_persists_an_opt_out() {
        for config in [
            Config::default(),
            toml::from_str::<Config>("provider = 'web'").unwrap(),
        ] {
            assert_eq!(serde_json::to_value(config).unwrap()["start_in_tray"], true);
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Storage {
            config: dir.path().join("config.toml"),
            history: dir.path().join("history.json"),
        };
        let config: Config = toml::from_str("start_in_tray = false").unwrap();
        store.save_config(&config).unwrap();
        assert_eq!(
            serde_json::to_value(store.load_config().unwrap()).unwrap()["start_in_tray"],
            false
        );
    }
    #[test]
    fn selected_style_defaults_to_simple_and_migrates_legacy_unselected() {
        use super::super::WritingStyle;
        assert_eq!(Config::default().write_style, WritingStyle::Simple);
        let old: Config = toml::from_str("write_style = 'default'").unwrap();
        assert_eq!(old.write_style, WritingStyle::Simple);
        for (name, expected) in [
            ("simple", WritingStyle::Simple),
            ("business", WritingStyle::Business),
            ("academic", WritingStyle::Academic),
            ("casual", WritingStyle::Casual),
        ] {
            let saved: Config = toml::from_str(&format!("write_style = '{name}'")).unwrap();
            assert_eq!(saved.write_style, expected);
            let reloaded: Config = toml::from_str(&toml::to_string(&saved).unwrap()).unwrap();
            assert_eq!(reloaded.write_style, expected);
        }
    }
    #[test]
    fn quick_placement_defaults_to_remembering_and_accepts_saved_negative_coordinates() {
        let value = serde_json::to_value(Config::default()).unwrap();
        assert_eq!(value["quick_position_policy"], "remember_last");
        let mut value = value;
        value["quick_position_policy"] = serde_json::json!("centered");
        value["quick_position"] = serde_json::json!({"x":-1500,"y":160});
        let config: Config = serde_json::from_value(value).unwrap();
        let saved = serde_json::to_value(config).unwrap();
        assert_eq!(saved["quick_position"]["x"], -1500);
    }
    #[test]
    fn old_config_migrates_and_modes_survive_restart() {
        let mut config: Config =
            toml::from_str("provider = 'web'\nsource = 'AUTO'\ntarget = 'FR'\n").unwrap();
        assert_eq!(config.initial_quick_mode(), WorkMode::Translate);
        config.last_mode = WorkMode::Write;
        let reloaded: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(reloaded.initial_quick_mode(), WorkMode::Write);
        config.quick_mode_policy = QuickModePolicy::AlwaysTranslate;
        assert_eq!(config.initial_quick_mode(), WorkMode::Translate);
        config.quick_mode_policy = QuickModePolicy::AlwaysWrite;
        config.last_mode = WorkMode::Translate;
        assert_eq!(config.initial_quick_mode(), WorkMode::Write);
        config.quick_mode_policy = QuickModePolicy::RememberLast;
        assert_eq!(config.initial_quick_mode(), WorkMode::Translate);
        assert_eq!(config.target, "FR");
        assert_eq!(config.write_language, "DE");
        assert_eq!(config.ui_language, "de");
        assert_eq!(config.write_style, super::super::WritingStyle::Simple);
        config.write_style = super::super::WritingStyle::Academic;
        let saved: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(saved.write_style, super::super::WritingStyle::Academic);
    }
    #[test]
    fn config_roundtrip_and_atomic_replace() {
        let dir = tempfile::tempdir().unwrap();
        let store = Storage {
            config: dir.path().join("config.toml"),
            history: dir.path().join("history.json"),
        };
        let mut c = store.load_config().unwrap();
        let id = c.instance_id;
        c.provider = ProviderMode::Web;
        store.save_config(&c).unwrap();
        assert_eq!(store.load_config().unwrap().provider, ProviderMode::Web);
        assert_eq!(store.load_config().unwrap().instance_id, id);
        c.hotkey_ms = 0;
        assert!(store.save_config(&c).is_err());
        assert!(
            !std::fs::read_to_string(store.config)
                .unwrap()
                .contains("api_key")
        );
    }
    #[test]
    fn all_six_ui_languages_persist_and_unknown_languages_are_rejected() {
        let mut config = Config::default();
        for language in ["en", "de", "cs", "fr", "ja", "it"] {
            config.ui_language = language.into();
            assert!(config.validate().is_ok());
            let loaded: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
            assert_eq!(loaded.ui_language, language);
            assert_eq!(loaded.target, "DE");
        }
        config.ui_language = "unknown".into();
        assert!(config.validate().is_err());
    }
}
