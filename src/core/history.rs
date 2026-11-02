use super::{
    Translation, TranslationRequest,
    config::{Storage, WorkMode, atomic_write},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: uuid::Uuid,
    pub created_at: u64,
    pub request: TranslationRequest,
    pub translation: Translation,
    pub favorite: bool,
    #[serde(default)]
    pub mode: WorkMode,
}
pub const MAX_ENTRIES: usize = 500;
const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const READ_ONLY_NOTICE: &str = "Der Verlauf konnte nicht geladen oder gesichert werden. Er bleibt unverändert; Speichern ist für diese Sitzung deaktiviert.";
pub const RECOVERY_NOTICE: &str = "Der Verlauf war beschädigt oder zu groß und wurde separat gesichert. Ein neuer Verlauf wurde gestartet.";

#[derive(Debug, thiserror::Error)]
#[error("Verlauf ist ungültig oder überschreitet die Größenlimits.")]
struct InvalidHistory;
fn entry_bytes(entry: &HistoryEntry) -> usize {
    [
        &entry.request.text,
        &entry.translation.text,
        &entry.request.source,
        &entry.request.target,
        &entry.translation.detected_source,
    ]
    .iter()
    .fold(0usize, |sum, value| sum.saturating_add(value.len()))
}
fn validate_entry(entry: &HistoryEntry) -> Result<()> {
    anyhow::ensure!(
        entry_bytes(entry) <= MAX_ENTRY_BYTES && entry.translation.detected_source.len() <= 32,
        InvalidHistory
    );
    entry.request.validate().map_err(|_| InvalidHistory)?;
    Ok(())
}
fn validate_entries(entries: &[HistoryEntry]) -> Result<()> {
    anyhow::ensure!(entries.len() <= MAX_ENTRIES, InvalidHistory);
    let mut total = 0usize;
    for entry in entries {
        validate_entry(entry)?;
        total = total.saturating_add(entry_bytes(entry));
        anyhow::ensure!(total <= MAX_TEXT_BYTES, InvalidHistory);
    }
    Ok(())
}
struct Entries(Vec<HistoryEntry>);
impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Entries;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a bounded translation history")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Entries, A::Error> {
                let mut entries = Vec::new();
                let mut total = 0usize;
                while let Some(entry) = seq.next_element::<HistoryEntry>()? {
                    validate_entry(&entry).map_err(serde::de::Error::custom)?;
                    total = total.saturating_add(entry_bytes(&entry));
                    if entries.len() >= MAX_ENTRIES || total > MAX_TEXT_BYTES {
                        return Err(serde::de::Error::custom(InvalidHistory));
                    }
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}
pub fn load(store: &Storage) -> Result<Vec<HistoryEntry>> {
    use std::io::Read;
    let metadata = match store.history.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    // Do not open special files or follow a known symlink in the profile.
    anyhow::ensure!(
        metadata.is_file(),
        std::io::Error::other("History is not a regular file")
    );
    let file = std::fs::File::open(&store.history)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        std::io::Error::other("History is not a regular file")
    );
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_FILE_BYTES as u64,
        InvalidHistory
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= MAX_FILE_BYTES, InvalidHistory);
    let entries: Entries = serde_json::from_slice(&bytes).map_err(|_| InvalidHistory)?;
    Ok(entries.0)
}
pub struct StartupHistory {
    pub entries: Vec<HistoryEntry>,
    pub writable: bool,
    pub notice: Option<&'static str>,
}
pub fn load_startup(store: &Storage) -> StartupHistory {
    match load(store) {
        Ok(entries) => StartupHistory {
            entries,
            writable: true,
            notice: None,
        },
        Err(error) => {
            // Preserve the original without reading/copying an arbitrarily large
            // damaged file. Recovery files are never automatically removed.
            let recovered = error.is::<InvalidHistory>()
                && store.history.parent().is_some_and(|parent| {
                    let destination =
                        parent.join(format!("history.recovery-{}.json", uuid::Uuid::new_v4()));
                    std::fs::rename(&store.history, destination).is_ok()
                });
            StartupHistory {
                entries: Vec::new(),
                writable: recovered,
                notice: Some(if recovered {
                    RECOVERY_NOTICE
                } else {
                    READ_ONLY_NOTICE
                }),
            }
        }
    }
}
struct BoundedJson(Vec<u8>);
impl std::io::Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_FILE_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(InvalidHistory));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn save(store: &Storage, entries: &[HistoryEntry]) -> Result<()> {
    validate_entries(entries)?;
    let mut bytes = BoundedJson(Vec::new());
    serde_json::to_writer_pretty(&mut bytes, entries)?;
    atomic_write(&store.history, &bytes.0)
}

pub fn remove(entries: &mut Vec<HistoryEntry>, id: uuid::Uuid) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    entries.len() != before
}

pub fn push(
    entries: &mut Vec<HistoryEntry>,
    request: TranslationRequest,
    translation: Translation,
    mode: WorkMode,
) -> bool {
    let entry = HistoryEntry {
        id: uuid::Uuid::new_v4(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        request,
        translation,
        favorite: false,
        mode,
    };
    if validate_entry(&entry).is_err() {
        return false;
    }
    entries.insert(0, entry);
    let mut total = 0usize;
    let mut count = 0usize;
    entries.retain(|entry| {
        let next = total.saturating_add(entry_bytes(entry));
        if count >= MAX_ENTRIES || next > MAX_TEXT_BYTES {
            return false;
        }
        total = next;
        count += 1;
        true
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removing_one_entry_preserves_order_favorites_and_persisted_neighbors() {
        let mut entries = Vec::new();
        for text in ["First", "Second", "Third"] {
            push(
                &mut entries,
                TranslationRequest {
                    text: text.into(),
                    source: "AUTO".into(),
                    target: "DE".into(),
                    style: Default::default(),
                },
                Translation {
                    text: text.into(),
                    detected_source: "EN".into(),
                },
                WorkMode::Translate,
            );
        }
        entries[0].favorite = true;
        let remove_id = entries[1].id;
        assert!(remove(&mut entries, remove_id));
        assert_eq!(
            entries
                .iter()
                .map(|e| e.request.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Third", "First"]
        );
        assert!(entries[0].favorite);
        assert!(!remove(&mut entries, remove_id));
        let dir = tempfile::tempdir().unwrap();
        let store = Storage {
            config: dir.path().join("config.toml"),
            history: dir.path().join("history.json"),
        };
        save(&store, &entries).unwrap();
        assert_eq!(load(&store).unwrap().len(), 2);
    }
}

#[cfg(test)]
mod security_regressions {
    use super::*;
    fn store(root: &tempfile::TempDir) -> Storage {
        Storage {
            config: root.path().join("config.toml"),
            history: root.path().join("history.json"),
        }
    }
    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            id: uuid::Uuid::new_v4(),
            created_at: 0,
            request: TranslationRequest {
                text: "Hello".into(),
                source: "EN".into(),
                target: "DE".into(),
                style: Default::default(),
            },
            translation: Translation {
                text: text.into(),
                detected_source: "EN".into(),
            },
            favorite: false,
            mode: Default::default(),
        }
    }
    #[test]
    fn rejects_oversized_or_invalid_history_on_load_and_save() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        for entries in [vec![entry("ok"); 501], vec![entry(&"x".repeat(65_537))]] {
            std::fs::write(&store.history, serde_json::to_vec(&entries).unwrap()).unwrap();
            assert!(load(&store).is_err(), "unbounded load");
            assert!(save(&store, &entries).is_err(), "unbounded save");
        }
        let mut invalid = entry("valid");
        invalid.request.target = "INVALID".into();
        std::fs::write(&store.history, serde_json::to_vec(&[invalid]).unwrap()).unwrap();
        assert!(load(&store).is_err());
        std::fs::File::create(&store.history)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert!(load(&store).is_err());
    }
    #[test]
    fn keeps_newest_entries_under_a_total_text_budget() {
        let mut entries = Vec::new();
        for i in 0..100 {
            let text = format!("{i}:{}", "x".repeat(60_000));
            push(
                &mut entries,
                entry("unused").request,
                Translation {
                    text,
                    detected_source: "EN".into(),
                },
                Default::default(),
            );
        }
        assert!(entries.len() < 100, "history text budget not enforced");
        assert!(entries[0].translation.text.starts_with("99:"));
        let before = entries[0].id;
        push(
            &mut entries,
            entry("unused").request,
            Translation {
                text: "x".repeat(100_000),
                detected_source: "EN".into(),
            },
            Default::default(),
        );
        assert_eq!(
            entries[0].id, before,
            "oversized result should not displace valid history"
        );
    }
}

#[cfg(test)]
mod recovery_regressions {
    use super::*;
    #[test]
    fn damaged_history_is_preserved_and_startup_can_continue() {
        let root = tempfile::tempdir().unwrap();
        let store = Storage {
            config: root.path().join("config.toml"),
            history: root.path().join("history.json"),
        };
        std::fs::write(&store.history, b"[damaged synthetic history").unwrap();
        let result = load_startup(&store);
        assert!(result.entries.is_empty());
        assert!(result.writable);
        assert!(result.notice.is_some());
        assert!(!store.history.exists());
        let copies: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(
            std::fs::read(&copies[0]).unwrap(),
            b"[damaged synthetic history"
        );
        save(&store, &[]).unwrap();
        assert!(load_startup(&store).notice.is_none());
        assert!(
            copies[0].exists(),
            "recovery file must never be pruned automatically"
        );
    }
    #[test]
    fn unreadable_history_does_not_block_startup_or_allow_overwriting_it() {
        let root = tempfile::tempdir().unwrap();
        let store = Storage {
            config: root.path().join("config.toml"),
            history: root.path().join("history.json"),
        };
        std::fs::create_dir(&store.history).unwrap();
        let result = load_startup(&store);
        assert!(result.entries.is_empty());
        assert!(!result.writable);
        assert!(result.notice.is_some());
        assert!(store.history.is_dir());
    }
}
