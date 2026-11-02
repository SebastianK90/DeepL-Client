# Build and verification

Commands run from the repository root. Use a current stable Rust toolchain (Rust 2024; at least 1.88 for the language syntax, with additional requirements imposed by Cargo.lock), Node.js 22+ and Git. The frontend itself has no npm build step. The supported packaged targets are Windows x64 and Linux x64.

`Cargo.lock` is included. PDFium setup downloads a fixed release with SHA-256 verification and prepares compressed engine/license resources under ignored `assets/pdfium/`. It requires `tar` on PATH. Runtime builds do not download executable code. Cross-compilation must use a separate PDFium asset directory through `--platform`, `--output` and `PDFIUM_ASSET_DIR`.

## Windows

Install Visual Studio Build Tools with Desktop development with C++, Windows SDK, stable Rust for MSVC, and Microsoft Edge WebView2 Runtime.

```powershell
node scripts/setup-pdfium.cjs
cargo run --locked --target x86_64-pc-windows-msvc
```

Release build:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc
node scripts/verify-windows-exe.cjs target/x86_64-pc-windows-msvc/release/deepl-client.exe
.\target\x86_64-pc-windows-msvc\release\deepl-client.exe --verify-pdf-engine
```

Distribute `target/x86_64-pc-windows-msvc/release/deepl-client.exe`. Keep `.cargo/config.toml`: it enables the static CRT used by the single-EXE build. The PE verifier rejects WebView2Loader, PDFium and additional VC/GNU runtime DLL imports. WebView2 Runtime and Windows system DLLs remain system prerequisites. PDFium is embedded and extracted to a private application cache when required. Windows OCR and fonts are supplied by Windows.

## Linux

Ubuntu 22.04 build and runtime packages (the CI workflow is the maintained packaging reference):

```sh
sudo apt-get update
sudo apt-get install -y build-essential curl file pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  patchelf libxdo-dev libfuse2 gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good gstreamer1.0-libav tesseract-ocr \
  tesseract-ocr-eng tesseract-ocr-deu tesseract-ocr-fra tesseract-ocr-ces \
  tesseract-ocr-ita tesseract-ocr-jpn fonts-dejavu-core
node scripts/setup-pdfium.cjs
cargo run --locked
```

KDE/Wayland screenshot and shortcut integration additionally require a functioning desktop portal and its KDE backend. Under X11 the client uses `xdotool` for selection replacement; install it if that feature is needed. The credential store requires a running Secret Service implementation. Media decoding depends on installed GStreamer plugins.

For Quick Assist position restoration and pointer placement on KDE Wayland, install XWayland (`xorg-xwayland` on Arch/CachyOS, `xwayland` on Ubuntu). When available, it is selected for the app's windows before GTK initializes. Keep `WAYLAND_DISPLAY` set so desktop portals and PRIMARY selection continue to use the Wayland session. An explicit `GDK_BACKEND` overrides this automatic selection.

For the supported AppImage packaging route:

```sh
cargo install tauri-cli --version '^2' --locked
bash scripts/build-appimage.sh
```

Output: `target/release/bundle/appimage/*.AppImage`. The script bundles OCR, six language models, DejaVu fonts and native media dependencies, then checks the extracted application, PDF engine, OCR initialization and media plugins. AppImage builds currently require a Debian/Ubuntu or Arch-compatible package inventory. On other Linux distributions, adapt the package names and validate the collected runtime files. FUSE is needed for normal AppImage startup; `APPIMAGE_EXTRACT_AND_RUN=1` can be used where FUSE is unavailable.

## Checks

After PDFium setup and system dependencies:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --workspace --no-default-features --bins --lib
cargo test --locked document::pdf::tests -- --ignored --skip replay_private --skip inspect_private --test-threads=1
node scripts/check-i18n.mjs
node scripts/check-glib-backport.cjs
node scripts/desktop-entry-check.cjs
```

The workspace test command selects the actual Tokio-based TTS configuration. Optional vendor runtimes, proxy features and vendor documentation examples are outside that check. PDF tests need the embedded engine and system replacement fonts. Ignored private inspection/replay helpers require explicitly supplied local paths and never call a network provider; they are intentionally skipped in CI.

Linux uses a source-compatible GLib 0.18.5 backport of RUSTSEC-2024-0429, selected through the root Cargo patch. Verify the reviewed source and resolved path with `node scripts/check-glib-backport.cjs`, and run `cargo test --release --locked glib_string_variant_iterator_handles_both_directions_and_unicode`. Optimization is necessary to reproduce the original bug. The upstream library code differs in only the two pointer-mutability lines documented in `vendor/glib/PATCHES.md`. A version-only advisory scan may still flag 0.18.5; the verified backport is the remediation, not an unrelated package upgrade or blanket advisory suppression.

Browser UI checks in `scripts/*-check.cjs` require Playwright and its Chromium browser. Install tooling in an ignored directory rather than adding runtime frontend dependencies:

```sh
npm install --prefix .tools/ui playwright
.tools/ui/node_modules/.bin/playwright install chromium
NODE_PATH="$PWD/.tools/ui/node_modules" node scripts/frontend-review-check.cjs
```

Some media checks additionally need explicit local audio fixtures. Screenshots generated by UI checks stay ignored. Service tests using live providers are separate from offline checks and transmit their test text to those services.

For native Quick Assist placement regressions, build the debug executable with `cargo build --locked`, then run `python3 scripts/linux-placement-check.py` in a KDE/XWayland desktop session with `xdotool` and Python GObject/GTK bindings installed. This uses isolated profiles under `.tools`, checks centered placement after moving/reopening and resizing, and verifies pointer/remembered placement against an independent compositor query. Do not use XWayland's `xdotool getmouselocation` as the pointer oracle: those coordinates can remain stale while the pointer is over native Wayland surfaces. The fixture never reads the clipboard or calls a translation service, and its entry point is excluded from release builds.

The KDE pointer query uses `workspace.cursorPos`, output geometry and `callDBus` from the [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/). It uses a private temporary file and a unique D-Bus callback, validates the KWin sender, bounds reply time/payload size, and unloads only its own script after the query.

## Security validation

Use the pinned Rust toolchain in `rust-toolchain.toml`. Workflow Actions and
Node/Tauri packager versions are fixed; see `.github/SECURITY_CHECKS.md`.
Run `node scripts/security-supply-chain.cjs`,
`node --test scripts/security-*.test.cjs`,
`node scripts/security-dependencies.cjs` and `node scripts/pdf-worker-check.cjs`.

Both platforms should run the release executable with `--verify-pdf-engine`
and `--verify-pdf-worker`. On native Windows/NTFS also run
`--verify-export-origin`. These modes use only synthetic fixtures, never a
real clipboard, API key or document. Windows origin/Job Object enforcement
cannot be validated positively through Wine on this test host; native CI
contains the required tests. Linux PDF processing requires kernel 5.11+ for
close_range(CLOEXEC); there is no silent unrestricted worker fallback.
