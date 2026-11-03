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

## Quick Assist layout compatibility

Quick Assist uses a viewport-sized flex layout (`100vw` × `100vh`) with controls measured in CSS pixels. Its default window size is 560 × 340 logical pixels; the native minimum is 492 × 292. Reopening preserves the current size within the session and clamps placement to the monitor work area. Native Windows resizing retains the same minimum.

The initial v0.1.4 build combined `zoom: 0.666666667` with `150vw`/`150vh`. The same frontend showed all controls on the local WebKitGTK 2.54.2 runtime, but clipped the right title controls and bottom actions with the AppImage's WebKitGTK 2.50.4 runtime. A larger minimum alone would not fix that viewport-relative scaling error. The correction removes CSS zoom and expresses the existing design's dimensions directly at the intended scale.

Validate runtime-sensitive layout changes with the packaged GTK/WebKit libraries, not only the host browser or a tray-startup smoke test. The native layout check uses an isolated debug fixture in a disposable extracted AppDir. It verifies default geometry, attempts to resize below the minimum, and checks the resulting native screenshot for the action row and title controls. Production artifacts must always retain the release binary, without debug fixture hooks.

The v0.1.4 correction was reproduced before the fix (the action-row check failed with the bundled runtime), passed after the fix, and was confirmed by the user. The browser regression also covers resize directions, visible control bounds, themes, position indicators and language-popup behavior. Native layout fixture output contains no clipboard contents or translated user text.
