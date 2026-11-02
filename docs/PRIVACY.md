# Data handling

- Translation sends source text to the selected DeepL API or experimental web provider. API Free/Pro uses a user-supplied key.
- Write, contextual alternatives and dictionary lookup use external DeepL services. Interactive alternatives and dictionary operations may use the web service even when translation uses the API.
- Text-to-speech sends the selected text to Microsoft Edge's speech service. Audio is held in memory for playback.
- Image OCR and screenshot OCR run locally. The recognized text is then used by the translation flow. Images are not uploaded by the OCR path.
- PDF, DOCX and PPTX parsing happens locally; extracted text is sent for translation. Source documents are not uploaded as whole files by this document path. Saving creates a translated output; source files remain unchanged. PDF parsing runs in a separate resource-limited process with private pipes; the parent sends extracted text for translation. Windows exports receive an Internet-zone origin marker because source provenance is unknown.
- History and favorites are local application data and can contain the texts you translate. Disable history or clear it in the application when needed. Damaged or oversized history is preserved beside the original as `history.recovery-<UUID>.json`; these recovery files also contain private text and are not removed by clearing the current history. Remove recovery files manually when no longer needed.
- API credentials use Windows Credential Manager or Linux Secret Service, not the example configuration or frontend bootstrap.
- Clipboard/selection shortcuts read local text. Wayland PRIMARY behavior depends on the source application; it can outlive a visible selection. Quick Assist does not silently fall back to ordinary clipboard contents when PRIMARY is unavailable.
- A Linux screenshot portal can create a temporary image file. The client removes the newly supplied portal file after reading it. Screenshots use a local area selector before OCR.

Configuration and history locations are resolved through `directories::ProjectDirs` with the application identity `dev / Local / DeepLClient`; they are outside the source checkout. Typical Linux locations are `~/.config/deeplclient/config.toml` and `~/.local/share/deeplclient/history.json`. XDG environment settings may change those paths.

The repository excludes real configuration, environment files, local history, private documents and historical test evidence. Bug reports should use synthetic examples and redact account information, document text and credentials.
