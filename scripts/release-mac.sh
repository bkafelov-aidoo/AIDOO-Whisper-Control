#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

key_path="${TAURI_UPDATER_KEY_PATH:-$HOME/Library/Application Support/AIDOO Release Keys/Whisper Lite/updater.key}"
if [[ ! -f "$key_path" ]]; then
  echo "Липсва updater signing key: $key_path" >&2
  exit 1
fi
export TAURI_SIGNING_PRIVATE_KEY="$key_path"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-$(security find-generic-password -a updater-key-password -s app.aidoo.whisper-lite.release -w)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/aidoo-whisper-lite-release-target}"
signing_identity="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)}"

python3 scripts/generate-third-party-notices.py
npm run check
cargo test --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml
cargo clippy --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npx tauri build --target aarch64-apple-darwin --bundles app,dmg

dmg="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/dmg" -maxdepth 1 -name '*.dmg' -print -quit)"
app="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos" -maxdepth 1 -name '*.app' -print -quit)"
archive="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos" -maxdepth 1 -name '*.app.tar.gz' -print -quit)"
signature="$archive.sig"
test -f "$dmg" && test -d "$app" && test -f "$archive" && test -f "$signature"
"$project_root/scripts/notarize-app-mac.sh" "$app"
file "$app/Contents/MacOS/aidoo-whisper-lite" | grep -q 'arm64'
test "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" = 'app.aidoo.whisper-lite'
test "$(plutil -extract LSMinimumSystemVersion raw "$app/Contents/Info.plist")" = '13.0'

# The first DMG was produced before the Keychain-profile notarization above. Rebuild it from
# the stapled app so offline Gatekeeper validation succeeds for the copy users install.
dmg_staging="$(mktemp -d -t aidoo-whisper-lite-dmg)"
verify_mount="$(mktemp -d -t aidoo-whisper-lite-mount)"
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$verify_mount" -quiet || true
  fi
  python3 - "$dmg_staging" "$verify_mount" <<'PY'
from pathlib import Path
import shutil, sys
for value in sys.argv[1:]:
    path = Path(value)
    if path.exists():
        shutil.rmtree(path)
PY
}
trap cleanup EXIT
ditto "$app" "$dmg_staging/$(basename "$app")"
ln -s /Applications "$dmg_staging/Applications"
python3 - "$dmg" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink(missing_ok=True)
PY
hdiutil create -volname "AIDOO Whisper Lite" -srcfolder "$dmg_staging" -ov -format UDZO "$dmg"
codesign --force --sign "$signing_identity" --timestamp "$dmg"

COPYFILE_DISABLE=1 tar -czf "$archive" -C "$(dirname "$app")" "$(basename "$app")"
env -u TAURI_SIGNING_PRIVATE_KEY npx tauri signer sign --private-key-path "$key_path" --password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" "$archive"
test -f "$signature"
"$project_root/scripts/notarize-mac.sh" "$dmg"

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$verify_mount" -quiet
mounted=true
installed_app="$verify_mount/$(basename "$app")"
codesign --verify --deep --strict --verbose=2 "$installed_app"
xcrun stapler validate "$installed_app"
spctl --assess --verbose=2 --type execute "$installed_app"
hdiutil detach "$verify_mount" -quiet
mounted=false

archive_check="$(mktemp -d -t aidoo-whisper-lite-updater)"
COPYFILE_DISABLE=1 tar -xzf "$archive" -C "$archive_check"
archived_app="$archive_check/$(basename "$app")"
codesign --verify --deep --strict --verbose=2 "$archived_app"
xcrun stapler validate "$archived_app"
spctl --assess --verbose=2 --type execute "$archived_app"
python3 - "$archive_check" <<'PY'
from pathlib import Path
import shutil, sys
path = Path(sys.argv[1])
if path.exists():
    shutil.rmtree(path)
PY

version="$(node -p "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8')).version")"
repo="${GITHUB_REPOSITORY:-bkafelov-aidoo/Aidoo-Whisper}"
asset_name="$(basename "$archive")"
pub_date="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
VERSION="$version" REPO="$repo" ASSET_NAME="$asset_name" PUB_DATE="$pub_date" SIGNATURE_PATH="$signature" node <<'NODE'
const fs = require('fs');
const metadata = {
  version: process.env.VERSION,
  notes: `AIDOO Whisper Lite ${process.env.VERSION}`,
  pub_date: process.env.PUB_DATE,
  platforms: {
    'darwin-aarch64': {
      signature: fs.readFileSync(process.env.SIGNATURE_PATH, 'utf8').trim(),
      url: `https://github.com/${process.env.REPO}/releases/download/lite-v${process.env.VERSION}/${encodeURIComponent(process.env.ASSET_NAME)}`
    }
  }
};
fs.writeFileSync('latest-lite.json', JSON.stringify(metadata, null, 2) + '\n');
NODE

printf 'Release assets:\n%s\n%s\n%s\n%s\n%s\n' "$dmg" "$dmg.sha256" "$archive" "$signature" "$project_root/latest-lite.json"
