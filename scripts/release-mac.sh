#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/aidoo-whisper-lite-release-target}"
signing_identity="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)}"

python3 scripts/generate-third-party-notices.py
npm run check
cargo test --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml
cargo clippy --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npx tauri build --target aarch64-apple-darwin --bundles app,dmg

dmg="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/dmg" -maxdepth 1 -name '*.dmg' -print -quit)"
app="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos" -maxdepth 1 -name '*.app' -print -quit)"
test -f "$dmg" && test -d "$app"
"$project_root/scripts/notarize-app-mac.sh" "$app"
file "$app/Contents/MacOS/aidoo-whisper-lite" | grep -q 'arm64'
test "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" = 'app.aidoo.whisper-lite'
test "$(plutil -extract LSMinimumSystemVersion raw "$app/Contents/Info.plist")" = '13.0'

# Rebuild the disk image from the stapled application so offline Gatekeeper validation
# succeeds for the exact copy that users install from the AIDOO website.
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

"$project_root/scripts/notarize-mac.sh" "$dmg"

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$verify_mount" -quiet
mounted=true
installed_app="$verify_mount/$(basename "$app")"
codesign --verify --deep --strict --verbose=2 "$installed_app"
xcrun stapler validate "$installed_app"
spctl --assess --verbose=2 --type execute "$installed_app"
hdiutil detach "$verify_mount" -quiet
mounted=false

printf 'Release assets:\n%s\n%s\n' "$dmg" "$dmg.sha256"
