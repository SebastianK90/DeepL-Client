#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Linux ]]; then
  echo 'AppImages müssen auf Linux gebaut werden.' >&2
  exit 1
fi
# Ubuntu 22.04 build packages are listed in .github/workflows/build.yml.
# Tauri's linuxdeploy scans existing AppDir ELF files and collects their native
# dependencies. Its GTK and GStreamer plugins supply WebKitGTK/media integration.
# https://github.com/linuxdeploy/linuxdeploy/blob/master/src/main.cpp
# https://v2.tauri.app/reference/config/#appimageconfig
command -v node >/dev/null
command -v tesseract >/dev/null
mkdir -p .tools
export APPIMAGE_EXTRACT_AND_RUN=1
export NO_STRIP=1
node <<'NODE'
const fs = require('node:fs');
const {execFileSync, spawnSync} = require('node:child_process');
const files = {};
const debian = spawnSync('dpkg-query', ['--version']).status === 0;
const arch = !debian && spawnSync('pacman', ['--version']).status === 0;
if (!debian && !arch) throw new Error('Packaging requires dpkg-query or pacman.');
const languages = ['eng', 'deu', 'fra', 'ces', 'ita', 'jpn'];
const languagePackage = lang => (debian ? 'tesseract-ocr-' : 'tesseract-data-') + lang;
const packageFiles = pkg => execFileSync(debian ? 'dpkg-query' : 'pacman',
  debian ? ['-L', pkg] : ['-Qlq', pkg], {encoding: 'utf8'}).split('\n');
const binary = execFileSync('which', ['tesseract'], {encoding: 'utf8'}).trim();
files['/usr/bin/tesseract'] = fs.realpathSync(binary);
for (const lang of languages) {
  const candidates = packageFiles(languagePackage(lang))
    .filter(p => p.endsWith('/' + lang + '.traineddata') && fs.existsSync(p));
  if (candidates.length !== 1) throw new Error('Missing/ambiguous traineddata for ' + lang);
  files['/usr/share/deepl-client/tessdata/' + lang + '.traineddata'] = candidates[0];
}
for (const pkg of [debian ? 'tesseract-ocr' : 'tesseract', ...languages.map(languagePackage)]) {
  const license = '/usr/share/doc/' + pkg + '/copyright';
  if (fs.existsSync(license)) files['/usr/share/doc/deepl-client/ocr/' + pkg + '-copyright'] = license;
  for (const notice of packageFiles(pkg).filter(p => p.startsWith('/usr/share/licenses/') && fs.existsSync(p) && fs.statSync(p).isFile())) {
    files['/usr/share/doc/deepl-client/ocr/' + pkg + '-' + require('node:path').basename(notice)] = notice;
  }
}
for (const name of ['DejaVuSans.ttf', 'DejaVuSans-Bold.ttf', 'DejaVuSans-Oblique.ttf', 'DejaVuSans-BoldOblique.ttf']) {
  const font = (debian ? '/usr/share/fonts/truetype/dejavu/' : '/usr/share/fonts/TTF/') + name;
  if (!fs.existsSync(font)) throw new Error('Missing PDF font: ' + font);
  files['/usr/share/fonts/truetype/dejavu/' + name] = font;
}
files['/usr/share/doc/deepl-client/fonts/copyright'] = debian
  ? '/usr/share/doc/fonts-dejavu-core/copyright' : '/usr/share/licenses/ttf-dejavu/LICENSE';
fs.writeFileSync('.tools/appimage-ocr.conf.json', JSON.stringify({bundle: {linux: {appimage: {files}}}}, null, 2));
NODE
cargo tauri build --bundles appimage --config .tools/appimage-ocr.conf.json -- --locked

# Test actual packaged files; extraction is verification, users get one AppImage.
verify_dir="$(mktemp -d "$PWD/.tools/appimage-check.XXXXXX")"
trap 'rm -rf -- "$verify_dir"' EXIT
mapfile -t images < <(find "$PWD/target/release/bundle/appimage" -maxdepth 1 -name '*.AppImage' -type f)
if [[ "${#images[@]}" -ne 1 ]]; then
  echo 'Exactly one AppImage is required; clean old bundle outputs first.' >&2
  exit 1
fi
(cd "$verify_dir" && "${images[0]}" --appimage-extract >/dev/null)
appdir="$verify_dir/squashfs-root"
bin="$appdir/usr/bin"
tessdata="$appdir/usr/share/deepl-client/tessdata"
# linuxdeploy rewrites RPATH, so checks do not use host LD_LIBRARY_PATH.
env -u APPDIR -u LD_LIBRARY_PATH "$bin/deepl-client" --verify-pdf-engine
env -u APPDIR -u LD_LIBRARY_PATH "$bin/deepl-client" --verify-pdf-worker
env -u APPDIR -u LD_LIBRARY_PATH "$bin/tesseract" --version
for lang in eng deu fra ces ita jpn; do test -s "$tessdata/$lang.traineddata"; done
for font in DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSans-BoldOblique.ttf; do test -s "$appdir/usr/share/fonts/truetype/dejavu/$font"; done
# Exercise model initialization and OCR with a generated grayscale PGM.
node - "$verify_dir/ocr-fixture.pgm" <<'NODE'
const fs = require('node:fs');
fs.writeFileSync(process.argv[2], Buffer.concat([Buffer.from('P5\n100 100\n255\n'), Buffer.alloc(10000, 255)]));
NODE
env -u APPDIR -u LD_LIBRARY_PATH "$bin/tesseract" "$verify_dir/ocr-fixture.pgm" stdout --tessdata-dir "$tessdata" -l eng+deu+fra+ces+ita+jpn --psm 6 > "$verify_dir/ocr-result.txt"
for executable in "$bin/deepl-client" "$bin/tesseract"; do
  if env -u LD_LIBRARY_PATH ldd "$executable" | grep -q 'not found'; then
    echo "Missing bundled native dependency: $executable" >&2
    exit 1
  fi
done
if ! find "$appdir/usr" -name 'libgstplayback.so' -print -quit | grep -q .; then
  echo 'GStreamer playback plugin missing from AppImage.' >&2
  exit 1
fi
if ! find "$appdir/usr" -name 'libgstmpg123.so' -print -quit | grep -q .; then
  echo 'GStreamer MP3 decoder missing from AppImage.' >&2
  exit 1
fi
printf 'Verified AppImage: %s\n' "${images[0]}"
