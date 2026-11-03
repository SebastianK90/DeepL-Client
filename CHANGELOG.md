# Changelog

## Unreleased

- Update reviewed GitHub Actions and Rust dependencies; migrate Linux portal calls and keep WebSocket/TLS versions compatible across the application and speech client.

## 0.1.3

- Fix the Windows PDF-worker self-test to accept PDFium’s non-breaking-space mapping while still rejecting incorrect translation text.
- Localize the PDF text-fit error in all six interface languages without changing PDF layout handling.

- Add scheduled advisory scanning, verified dependency exceptions, pinned Actions and exact build tool versions; enable repository security alerts and secret protection.
- Mark Windows translated exports as untrusted before atomic publication; refuse destinations without origin metadata support.
- Move PDF parsing/rewriting into a resource-limited subprocess, keeping translation and credentials in the parent and enforcing cancellation, bounded IPC and cleanup.

- Preflight Office packages before translation: reject active/embedded payloads, unsafe relationships and ambiguous ZIP paths; retain ordinary links, images, page fields and bookmark links.
- Bound history loading and storage; preserve damaged or oversized originals in separate recovery files and continue startup with safe read-only fallback when recovery is unavailable.

- Restrict all Tauri commands to their required local windows; keep history and general settings inaccessible to Quick Assist and auxiliary windows.

- Backport the official GLib 0.18.5 string-variant iterator fix for RUSTSEC-2024-0429, preserving the GTK3/Tauri interfaces. Add optimized regression coverage and verify that the full dependency graph resolves to the reviewed patched source.

## 0.1.2

- Resize Quick Assist from every edge and corner. Windows resizing uses the same focus protection as dragging, so the overlay stays open during the gesture.
- Preserve the current Quick Assist size when reopening, with a minimum size that keeps its controls accessible.
- Show a placement-policy icon in the title bar, with localized tooltips and live settings updates.
- Use XWayland for KDE window placement when available, while preserving Wayland shortcuts, PRIMARY selection and screenshot portals.
- Reapply the selected placement after KDE maps the window, fixing centering after moving and reopening it.
- Read the real KDE pointer through a temporary compositor query and map output coordinates to XWayland, avoiding stale X11 pointer coordinates over native Wayland applications.
- Select the language-menu implementation using the actual window backend. Open above or below according to available space and mirror the search field toward Quick Assist.
- Add regression checks for all resize directions, placement indicators, native KDE positioning and compositor coordinate conversion.
