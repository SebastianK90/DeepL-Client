#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Linux ]] || { echo 'Linux erforderlich.' >&2; exit 1; }
command -v appimagetool >/dev/null || { echo 'appimagetool muss im PATH liegen.' >&2; exit 1; }
# Alternative requested cargo-appimage route. Requires manual validation of
# WebKit helper executables, GTK modules and runtime data on the target system.
# Production packaging should use build-appimage.sh (Tauri/linuxdeploy).
export APPIMAGE_EXTRACT_AND_RUN=1
cargo appimage --release --locked
