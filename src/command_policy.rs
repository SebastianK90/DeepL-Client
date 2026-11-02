//! Exact command-to-window policy shared by build-time ACL enrollment and runtime dispatch.
macro_rules! commands {
    ($(($command:literal, [$($window:literal),+])),* $(,)?) => {
        #[allow(dead_code)]
        pub const COMMANDS: &[&str] = &[$($command),*];
        const RULES: &[(&str, &[&str])] = &[$(($command, &[$($window),+])),*];
    };
}
commands! {
    ("save_api_key", ["main"]),
    ("delete_api_key", ["main"]),
    ("toggle_favorite", ["main"]),
    ("clear_history", ["main"]),
    ("delete_history_entry", ["main"]),
    ("read_clipboard", ["main"]),
    ("share_text", ["main"]),
    ("capture_screenshot", ["main"]),
    ("recognize_image", ["main"]),
    ("translate_document", ["main"]),
    ("cancel_document", ["main"]),
    ("save_document", ["main"]),
    ("third_party_licenses", ["main"]),
    ("main_window_action", ["main"]),
    ("begin_speech_session", ["main"]),
    ("cancel_speech", ["main"]),
    ("synthesize_speech", ["main"]),
    ("stream_speech", ["main"]),
    ("lookup_alternatives", ["main"]),
    ("apply_word_alternative", ["main"]),
    ("lookup_dictionary", ["main"]),
    ("bootstrap", ["main", "quick"]),
    ("translate", ["main", "quick"]),
    ("write", ["main", "quick"]),
    ("remember_mode", ["main", "quick"]),
    ("save_settings", ["main", "quick"]),
    ("copy_text", ["main", "quick"]),
    ("begin_quick", ["quick"]),
    ("replace_selection", ["quick"]),
    ("hide_quick", ["quick"]),
    ("open_main", ["quick"]),
    ("start_quick_drag", ["quick"]),
    ("start_quick_resize", ["quick"]),
    ("open_language_popup", ["quick"]),
    ("language_popup_state", ["quick-language"]),
    ("choose_quick_language", ["quick-language"]),
    ("resize_language_popup", ["quick-language"]),
    ("close_language_popup", ["quick", "quick-language"]),
    ("capture_area_state", ["capture-select"]),
    ("capture_area_finish", ["capture-select"]),
}
pub fn allows(command: &str, window: &str) -> bool {
    RULES
        .iter()
        .any(|(name, windows)| *name == command && windows.contains(&window))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_and_runtime_policy_agree_for_every_command_and_window() {
        let files = [
            include_str!("../capabilities/main.json"),
            include_str!("../capabilities/quick.json"),
            include_str!("../capabilities/language.json"),
            include_str!("../capabilities/capture.json"),
        ];
        let capabilities: Vec<serde_json::Value> = files
            .iter()
            .map(|f| serde_json::from_str(f).unwrap())
            .collect();
        for window in [
            "main",
            "quick",
            "quick-language",
            "capture-select",
            "untrusted",
            "",
            "main-child",
        ] {
            for command in COMMANDS
                .iter()
                .copied()
                .chain(["unknown", "plugin:shell|execute"])
            {
                let permission = format!("allow-{}", command.replace('_', "-"));
                let granted = capabilities.iter().any(|cap| {
                    cap["windows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|w| w == window)
                        && cap["permissions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|p| p == &permission)
                });
                assert_eq!(allows(command, window), granted, "{window}: {command}");
            }
        }
    }
    #[test]
    fn tauri_resolves_the_generated_manifest_for_linux_and_windows() {
        use std::collections::BTreeMap;
        use tauri::utils::{
            acl::{
                ExecutionContext, capability::Capability, manifest::Manifest, resolved::Resolved,
            },
            platform::Target,
        };
        let manifests: BTreeMap<String, Manifest> =
            serde_json::from_str(include_str!("../gen/schemas/acl-manifests.json")).unwrap();
        for target in [Target::Linux, Target::Windows] {
            let capabilities: BTreeMap<String, Capability> =
                serde_json::from_str(include_str!("../gen/schemas/capabilities.json")).unwrap();
            let resolved = Resolved::resolve(&manifests, capabilities, target).unwrap();
            assert!(
                resolved.has_app_acl,
                "Custom app commands must not use the default unrestricted path"
            );
            for window in [
                "main",
                "quick",
                "quick-language",
                "capture-select",
                "untrusted",
                "main-child",
            ] {
                for command in COMMANDS.iter().copied().chain(["unknown"]) {
                    let granted = resolved
                        .allowed_commands
                        .get(command)
                        .is_some_and(|entries| {
                            entries.iter().any(|entry| {
                                matches!(entry.context, ExecutionContext::Local)
                                    && entry.windows.iter().any(|p| p.matches(window))
                            })
                        });
                    assert_eq!(
                        granted,
                        allows(command, window),
                        "{target:?} {window}: {command}"
                    );
                }
            }
            assert!(
                resolved
                    .allowed_commands
                    .values()
                    .flatten()
                    .all(|entry| matches!(entry.context, ExecutionContext::Local))
            );
        }
    }
    #[test]
    fn sensitive_commands_are_denied_outside_the_main_window() {
        for command in [
            "save_api_key",
            "delete_api_key",
            "clear_history",
            "toggle_favorite",
            "delete_history_entry",
            "read_clipboard",
            "translate_document",
            "capture_screenshot",
            "stream_speech",
        ] {
            assert!(allows(command, "main"));
            for window in ["quick", "quick-language", "capture-select", "unknown"] {
                assert!(!allows(command, window), "{window}: {command}");
            }
        }
        assert!(!allows("bootstrap", "quick-language"));
        assert!(!allows("bootstrap", "capture-select"));
        assert!(!allows("capture_area_state", "main"));
        assert!(!allows("begin_quick", "main"));
    }
}
