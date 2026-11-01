# Architecture

- `src/core`: configuration, history, shared language targets, session state and placement rules.
- `src/provider`: official API and experimental web translation/Write providers.
- `src/ui`: Tauri commands, windows, platform integration and Quick Assist orchestration.
- `src/document`: local PDF, DOCX and PPTX processing. PDF geometry is planned before page edits; independently positioned cells retain their coordinates.
- `src/capture`, `src/clipboard`, `src/hotkey`, `src/replacement`: platform-specific capture, selection, shortcut and replacement behavior.
- `src/speech`: voice selection and playback orchestration.
- `frontend`: static HTML, CSS and JavaScript; no framework build pipeline.
- `vendor/msedge-tts`: patched upstream speech transport, with its original license and local patch summary.
- `tests/fixtures`: synthetic PDF regression fixtures and the embedded font's license.
- `scripts/setup-pdfium.cjs`: hash-pinned build-time PDFium setup. `src/embedded_pdfium.rs` handles embedded engine extraction/loading and notices.
- `scripts/build-appimage.sh`: Linux runtime bundling and package verification.
- `.cargo/config.toml` and `build.rs`: Windows static CRT/loader integration.

Linux uses GTK for native file drops and the Quick Assist language popover, a GIO bridge for primary activation of its existing StatusNotifierItem (Tauri does not emit Linux tray click events), desktop portals on Wayland, Tesseract for OCR, and system/packaged media plugins. Windows uses native Windows APIs, Windows OCR and WebView2. Native capabilities differ by desktop and must be tested on each platform.

Tauri's frontend capability boundary, content security policy and per-window command checks constrain access to native operations. Long-running document and OCR paths have cancellation, size and concurrency limits. See the source and tests for the precise behavior; this is not a security certification.
