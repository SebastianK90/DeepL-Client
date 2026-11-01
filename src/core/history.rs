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
pub fn load(store: &Storage) -> Result<Vec<HistoryEntry>> {
    match std::fs::read(&store.history) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
pub fn save(store: &Storage, entries: &[HistoryEntry]) -> Result<()> {
    atomic_write(&store.history, &serde_json::to_vec_pretty(entries)?)
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
) {
    entries.insert(
        0,
        HistoryEntry {
            id: uuid::Uuid::new_v4(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            request,
            translation,
            favorite: false,
            mode,
        },
    );
    if entries.len() > 500 {
        entries.truncate(500);
    }
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
