use super::config::WorkMode;
use super::{Translation, TranslationRequest};

#[derive(Default)]
pub struct QuickSession {
    pub id: u64,
    pub original: String,
    pub result: Option<(TranslationRequest, Translation, WorkMode)>,
    pub consumed: bool,
}
impl QuickSession {
    pub fn start(&mut self, text: String) -> u64 {
        self.id = self.id.wrapping_add(1).max(1);
        self.original = text;
        self.result = None;
        self.consumed = false;
        self.id
    }
    pub fn complete(
        &mut self,
        id: u64,
        request: TranslationRequest,
        result: Translation,
        mode: WorkMode,
    ) {
        if self.id == id && self.original == request.text && !self.consumed {
            self.result = Some((request, result, mode));
        }
    }
    pub fn replacement(&self, id: u64, displayed: &str) -> Result<String, &'static str> {
        if id != self.id || self.consumed {
            return Err("Diese Quick-Assist-Sitzung ist nicht mehr aktuell.");
        }
        let (_, result, _) = self
            .result
            .as_ref()
            .ok_or("Kein aktuelles Ergebnis zum Ersetzen vorhanden.")?;
        if displayed != result.text {
            return Err("Das angezeigte Ergebnis ist nicht mehr aktuell.");
        }
        Ok(result.text.clone())
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn verify_selection(
    original: &str,
    observed: &str,
    session_current: bool,
) -> Result<(), &'static str> {
    if !session_current {
        return Err("Diese Quick-Assist-Sitzung ist nicht mehr aktuell.");
    }
    if original != observed || original.is_empty() {
        return Err("Die ursprüngliche Textauswahl wurde geändert. Bitte erneut Strg+C+C drücken.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(text: &str) -> TranslationRequest {
        serde_json::from_value(serde_json::json!({"text":text,"source":"AUTO","target":"DE"}))
            .unwrap()
    }
    #[test]
    fn old_sessions_and_unissued_results_cannot_be_pasted() {
        let mut session = QuickSession::default();
        let id = session.start("Hello".into());
        assert!(session.replacement(id, "Hallo").is_err());
        session.complete(
            id,
            request("Hello"),
            Translation {
                text: "Hallo".into(),
                detected_source: "EN".into(),
            },
            WorkMode::Translate,
        );
        assert_eq!(session.replacement(id, "Hallo").unwrap(), "Hallo");
        assert!(session.replacement(id, "Injected text").is_err());
        let next = session.start("New".into());
        session.complete(
            id,
            request("Hello"),
            Translation {
                text: "Old".into(),
                detected_source: "EN".into(),
            },
            WorkMode::Translate,
        );
        assert!(session.replacement(next, "Old").is_err());
        assert!(session.replacement(id, "Hallo").is_err());
    }
    #[test]
    fn changed_selection_and_repeated_replace_are_rejected() {
        assert!(verify_selection("Hello", "Different", true).is_err());
        assert!(verify_selection("Hello", "Hello", false).is_err());
        assert!(verify_selection("", "", true).is_err());
        assert!(verify_selection("Hello", "Hello", true).is_ok());
        let mut session = QuickSession::default();
        let id = session.start("Hello".into());
        session.complete(
            id,
            request("Hello"),
            Translation {
                text: "Hallo".into(),
                detected_source: "EN".into(),
            },
            WorkMode::Translate,
        );
        session.consumed = true;
        assert!(session.replacement(id, "Hallo").is_err());
    }
}
